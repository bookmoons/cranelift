//! Legalize instructions.
//!
//! A legal instruction is one that can be mapped directly to a machine code instruction for the
//! target ISA. The `legalize_function()` function takes as input any function and transforms it
//! into an equivalent function using only legal instructions.
//!
//! The characteristics of legal instructions depend on the target ISA, so any given instruction
//! can be legal for one ISA and illegal for another.
//!
//! Besides transforming instructions, the legalizer also fills out the `function.encodings` map
//! which provides a legal encoding recipe for every instruction.
//!
//! The legalizer does not deal with register allocation constraints. These constraints are derived
//! from the encoding recipes, and solved later by the register allocator.

use crate::bitset::BitSet;
use crate::cursor::{Cursor, FuncCursor};
use crate::flowgraph::ControlFlowGraph;
use crate::ir::types::I32;
use crate::ir::{self, InstBuilder, MemFlags};
use crate::isa::TargetIsa;
use crate::predicates;
use crate::timing;
use std::collections::BTreeSet;
use std::vec::Vec;

mod boundary;
mod call;
mod globalvalue;
mod heap;
mod libcall;
mod split;
mod table;

use self::call::expand_call;
use self::globalvalue::expand_global_value;
use self::heap::expand_heap_addr;
use self::libcall::expand_as_libcall;
use self::table::expand_table_addr;

enum LegalizeInstResult {
    Done,
    Legalized,
    SplitLegalizePending,
}

/// Legalize `inst` for `isa`.
fn legalize_inst(
    inst: ir::Inst,
    pos: &mut FuncCursor,
    cfg: &mut ControlFlowGraph,
    isa: &dyn TargetIsa,
) -> LegalizeInstResult {
    let opcode = pos.func.dfg[inst].opcode();

    // Check for ABI boundaries that need to be converted to the legalized signature.
    if opcode.is_call() {
        if boundary::handle_call_abi(inst, pos.func, cfg) {
            return LegalizeInstResult::Legalized;
        }
    } else if opcode.is_return() {
        if boundary::handle_return_abi(inst, pos.func, cfg) {
            return LegalizeInstResult::Legalized;
        }
    } else if opcode.is_branch() {
        split::simplify_branch_arguments(&mut pos.func.dfg, inst);
    } else if opcode == ir::Opcode::Isplit {
        pos.use_srcloc(inst);

        let arg = match pos.func.dfg[inst] {
            ir::InstructionData::Unary { arg, .. } => pos.func.dfg.resolve_aliases(arg),
            _ => panic!("Expected isplit: {}", pos.func.dfg.display_inst(inst, None)),
        };

        match pos.func.dfg.value_def(arg) {
            ir::ValueDef::Result(inst, _num) => {
                if let ir::InstructionData::Binary {
                    opcode: ir::Opcode::Iconcat,
                    ..
                } = pos.func.dfg[inst]
                {
                    // `arg` was created by an `iconcat` instruction.
                } else {
                    // `arg` was not created by an `iconcat` instruction. Don't try to resolve it,
                    // as otherwise `split::isplit` will re-insert the original `isplit`, causing
                    // an endless loop.
                    return LegalizeInstResult::SplitLegalizePending;
                }
            }
            ir::ValueDef::Param(_ebb, _num) => {}
        }

        let res = pos.func.dfg.inst_results(inst).to_vec();
        assert_eq!(res.len(), 2);
        let (resl, resh) = (res[0], res[1]); // Prevent borrowck error

        // Remove old isplit
        pos.func.dfg.clear_results(inst);
        pos.remove_inst();

        let curpos = pos.position();
        let srcloc = pos.srcloc();
        let (xl, xh) = split::isplit(pos.func, cfg, curpos, srcloc, arg);

        pos.func.dfg.change_to_alias(resl, xl);
        pos.func.dfg.change_to_alias(resh, xh);

        return LegalizeInstResult::Legalized;
    }

    match pos.func.update_encoding(inst, isa) {
        Ok(()) => LegalizeInstResult::Done,
        Err(action) => {
            // We should transform the instruction into legal equivalents.
            // If the current instruction was replaced, we need to double back and revisit
            // the expanded sequence. This is both to assign encodings and possible to
            // expand further.
            // There's a risk of infinite looping here if the legalization patterns are
            // unsound. Should we attempt to detect that?
            if action(inst, pos.func, cfg, isa) {
                return LegalizeInstResult::Legalized;
            }

            // We don't have any pattern expansion for this instruction either.
            // Try converting it to a library call as a last resort.
            if expand_as_libcall(inst, pos.func, isa) {
                LegalizeInstResult::Legalized
            } else {
                LegalizeInstResult::Done
            }
        }
    }
}

/// Legalize `func` for `isa`.
///
/// - Transform any instructions that don't have a legal representation in `isa`.
/// - Fill out `func.encodings`.
///
pub fn legalize_function(func: &mut ir::Function, cfg: &mut ControlFlowGraph, isa: &dyn TargetIsa) {
    let _tt = timing::legalize();
    debug_assert!(cfg.is_valid());

    boundary::legalize_signatures(func, isa);

    func.encodings.resize(func.dfg.num_insts());

    let mut pos = FuncCursor::new(func);

    // This must be a set to prevent trying to legalize `isplit` and `vsplit` twice in certain cases.
    let mut pending_splits = BTreeSet::new();

    // Process EBBs in layout order. Some legalization actions may split the current EBB or append
    // new ones to the end. We need to make sure we visit those new EBBs too.
    while let Some(ebb) = pos.next_ebb() {
        split::split_ebb_params(pos.func, cfg, ebb);

        // Keep track of the cursor position before the instruction being processed, so we can
        // double back when replacing instructions.
        let mut prev_pos = pos.position();

        while let Some(inst) = pos.next_inst() {
            match legalize_inst(inst, &mut pos, cfg, isa) {
                // Remember this position in case we need to double back.
                LegalizeInstResult::Done => prev_pos = pos.position(),

                // Go back and legalize the inserted return value conversion instructions.
                LegalizeInstResult::Legalized => pos.set_position(prev_pos),

                // The argument of a `isplit` or `vsplit` instruction didn't resolve to a
                // `iconcat` or `vconcat` instruction. Try again after legalizing the rest of
                // the instructions.
                LegalizeInstResult::SplitLegalizePending => {
                    pending_splits.insert(inst);
                }
            }
        }
    }

    // Try legalizing `isplit` and `vsplit` instructions, which could not previously be legalized.
    for inst in pending_splits {
        pos.goto_inst(inst);
        legalize_inst(inst, &mut pos, cfg, isa);
    }

    // Now that we've lowered all br_tables, we don't need the jump tables anymore.
    if !isa.flags().jump_tables_enabled() {
        pos.func.jump_tables.clear();
    }
}

// Include legalization patterns that were generated by `gen_legalizer.rs` from the
// `TransformGroup` in `cranelift-codegen/meta/shared/legalize.rs`.
//
// Concretely, this defines private functions `narrow()`, and `expand()`.
include!(concat!(env!("OUT_DIR"), "/legalizer.rs"));

/// Custom expansion for conditional trap instructions.
/// TODO: Add CFG support to the Rust DSL patterns so we won't have to do this.
fn expand_cond_trap(
    inst: ir::Inst,
    func: &mut ir::Function,
    cfg: &mut ControlFlowGraph,
    _isa: &dyn TargetIsa,
) {
    // Parse the instruction.
    let trapz;
    let (arg, code) = match func.dfg[inst] {
        ir::InstructionData::CondTrap { opcode, arg, code } => {
            // We want to branch *over* an unconditional trap.
            trapz = match opcode {
                ir::Opcode::Trapz => true,
                ir::Opcode::Trapnz => false,
                _ => panic!("Expected cond trap: {}", func.dfg.display_inst(inst, None)),
            };
            (arg, code)
        }
        _ => panic!("Expected cond trap: {}", func.dfg.display_inst(inst, None)),
    };

    // Split the EBB after `inst`:
    //
    //     trapnz arg
    //     ..
    //
    // Becomes:
    //
    //     brz arg, new_ebb_resume
    //     jump new_ebb_trap
    //
    //   new_ebb_trap:
    //     trap
    //
    //   new_ebb_resume:
    //     ..
    let old_ebb = func.layout.pp_ebb(inst);
    let new_ebb_trap = func.dfg.make_ebb();
    let new_ebb_resume = func.dfg.make_ebb();

    // Replace trap instruction by the inverted condition.
    if trapz {
        func.dfg.replace(inst).brnz(arg, new_ebb_resume, &[]);
    } else {
        func.dfg.replace(inst).brz(arg, new_ebb_resume, &[]);
    }

    // Add jump instruction after the inverted branch.
    let mut pos = FuncCursor::new(func).after_inst(inst);
    pos.use_srcloc(inst);
    pos.ins().jump(new_ebb_trap, &[]);

    // Insert the new label and the unconditional trap terminator.
    pos.insert_ebb(new_ebb_trap);
    pos.ins().trap(code);

    // Insert the new label and resume the execution when the trap fails.
    pos.insert_ebb(new_ebb_resume);

    // Finally update the CFG.
    cfg.recompute_ebb(pos.func, old_ebb);
    cfg.recompute_ebb(pos.func, new_ebb_resume);
    cfg.recompute_ebb(pos.func, new_ebb_trap);
}

/// Jump tables.
fn expand_br_table(
    inst: ir::Inst,
    func: &mut ir::Function,
    cfg: &mut ControlFlowGraph,
    isa: &dyn TargetIsa,
) {
    if isa.flags().jump_tables_enabled() {
        expand_br_table_jt(inst, func, cfg, isa);
    } else {
        expand_br_table_conds(inst, func, cfg, isa);
    }
}

/// Expand br_table to jump table.
fn expand_br_table_jt(
    inst: ir::Inst,
    func: &mut ir::Function,
    cfg: &mut ControlFlowGraph,
    isa: &dyn TargetIsa,
) {
    use crate::ir::condcodes::IntCC;

    let (arg, default_ebb, table) = match func.dfg[inst] {
        ir::InstructionData::BranchTable {
            opcode: ir::Opcode::BrTable,
            arg,
            destination,
            table,
        } => (arg, destination, table),
        _ => panic!("Expected br_table: {}", func.dfg.display_inst(inst, None)),
    };

    // Rewrite:
    //
    //     br_table $idx, default_ebb, $jt
    //
    // To:
    //
    //     $oob = ifcmp_imm $idx, len($jt)
    //     brif uge $oob, default_ebb
    //     jump fallthrough_ebb
    //
    //   fallthrough_ebb:
    //     $base = jump_table_base.i64 $jt
    //     $rel_addr = jump_table_entry.i64 $idx, $base, 4, $jt
    //     $addr = iadd $base, $rel_addr
    //     indirect_jump_table_br $addr, $jt

    let ebb = func.layout.pp_ebb(inst);
    let jump_table_ebb = func.dfg.make_ebb();

    let mut pos = FuncCursor::new(func).at_inst(inst);
    pos.use_srcloc(inst);

    // Bounds check.
    let table_size = pos.func.jump_tables[table].len() as i64;
    let oob = pos
        .ins()
        .icmp_imm(IntCC::UnsignedGreaterThanOrEqual, arg, table_size);

    pos.ins().brnz(oob, default_ebb, &[]);
    pos.ins().jump(jump_table_ebb, &[]);
    pos.insert_ebb(jump_table_ebb);

    let addr_ty = isa.pointer_type();

    let arg = if pos.func.dfg.value_type(arg) == addr_ty {
        arg
    } else {
        pos.ins().uextend(addr_ty, arg)
    };

    let base_addr = pos.ins().jump_table_base(addr_ty, table);
    let entry = pos
        .ins()
        .jump_table_entry(arg, base_addr, I32.bytes() as u8, table);

    let addr = pos.ins().iadd(base_addr, entry);
    pos.ins().indirect_jump_table_br(addr, table);

    pos.remove_inst();
    cfg.recompute_ebb(pos.func, ebb);
    cfg.recompute_ebb(pos.func, jump_table_ebb);
}

/// Expand br_table to series of conditionals.
fn expand_br_table_conds(
    inst: ir::Inst,
    func: &mut ir::Function,
    cfg: &mut ControlFlowGraph,
    _isa: &dyn TargetIsa,
) {
    use crate::ir::condcodes::IntCC;

    let (arg, default_ebb, table) = match func.dfg[inst] {
        ir::InstructionData::BranchTable {
            opcode: ir::Opcode::BrTable,
            arg,
            destination,
            table,
        } => (arg, destination, table),
        _ => panic!("Expected br_table: {}", func.dfg.display_inst(inst, None)),
    };

    let ebb = func.layout.pp_ebb(inst);

    // This is a poor man's jump table using just a sequence of conditional branches.
    let table_size = func.jump_tables[table].len();
    let mut cond_failed_ebb = vec![];
    if table_size >= 1 {
        cond_failed_ebb = std::vec::Vec::with_capacity(table_size - 1);
        for _ in 0..table_size - 1 {
            cond_failed_ebb.push(func.dfg.make_ebb());
        }
    }

    let mut pos = FuncCursor::new(func).at_inst(inst);
    pos.use_srcloc(inst);

    for i in 0..table_size {
        let dest = pos.func.jump_tables[table].as_slice()[i];
        let t = pos.ins().icmp_imm(IntCC::Equal, arg, i as i64);
        pos.ins().brnz(t, dest, &[]);
        // Jump to the next case.
        if i < table_size - 1 {
            pos.ins().jump(cond_failed_ebb[i], &[]);
            pos.insert_ebb(cond_failed_ebb[i]);
        }
    }

    // `br_table` jumps to the default destination if nothing matches
    pos.ins().jump(default_ebb, &[]);

    pos.remove_inst();
    cfg.recompute_ebb(pos.func, ebb);
    for failed_ebb in cond_failed_ebb.into_iter() {
        cfg.recompute_ebb(pos.func, failed_ebb);
    }
}

/// Expand the select instruction.
///
/// Conditional moves are available in some ISAs for some register classes. The remaining selects
/// are handled by a branch.
fn expand_select(
    inst: ir::Inst,
    func: &mut ir::Function,
    cfg: &mut ControlFlowGraph,
    _isa: &dyn TargetIsa,
) {
    let (ctrl, tval, fval) = match func.dfg[inst] {
        ir::InstructionData::Ternary {
            opcode: ir::Opcode::Select,
            args,
        } => (args[0], args[1], args[2]),
        _ => panic!("Expected select: {}", func.dfg.display_inst(inst, None)),
    };

    // Replace `result = select ctrl, tval, fval` with:
    //
    //   brnz ctrl, new_ebb(tval)
    //   jump new_ebb(fval)
    // new_ebb(result):
    let old_ebb = func.layout.pp_ebb(inst);
    let result = func.dfg.first_result(inst);
    func.dfg.clear_results(inst);
    let new_ebb = func.dfg.make_ebb();
    func.dfg.attach_ebb_param(new_ebb, result);

    func.dfg.replace(inst).brnz(ctrl, new_ebb, &[tval]);
    let mut pos = FuncCursor::new(func).after_inst(inst);
    pos.use_srcloc(inst);
    pos.ins().jump(new_ebb, &[fval]);
    pos.insert_ebb(new_ebb);

    cfg.recompute_ebb(pos.func, new_ebb);
    cfg.recompute_ebb(pos.func, old_ebb);
}

fn expand_br_icmp(
    inst: ir::Inst,
    func: &mut ir::Function,
    cfg: &mut ControlFlowGraph,
    _isa: &dyn TargetIsa,
) {
    let (cond, a, b, destination, ebb_args) = match func.dfg[inst] {
        ir::InstructionData::BranchIcmp {
            cond,
            destination,
            ref args,
            ..
        } => (
            cond,
            args.get(0, &func.dfg.value_lists).unwrap(),
            args.get(1, &func.dfg.value_lists).unwrap(),
            destination,
            args.as_slice(&func.dfg.value_lists)[2..].to_vec(),
        ),
        _ => panic!("Expected br_icmp {}", func.dfg.display_inst(inst, None)),
    };

    let old_ebb = func.layout.pp_ebb(inst);
    func.dfg.clear_results(inst);

    let icmp_res = func.dfg.replace(inst).icmp(cond, a, b);
    let mut pos = FuncCursor::new(func).after_inst(inst);
    pos.use_srcloc(inst);
    pos.ins().brnz(icmp_res, destination, &ebb_args);

    cfg.recompute_ebb(pos.func, destination);
    cfg.recompute_ebb(pos.func, old_ebb);
}

/// Expand illegal `f32const` and `f64const` instructions.
fn expand_fconst(
    inst: ir::Inst,
    func: &mut ir::Function,
    _cfg: &mut ControlFlowGraph,
    _isa: &dyn TargetIsa,
) {
    let ty = func.dfg.value_type(func.dfg.first_result(inst));
    debug_assert!(!ty.is_vector(), "Only scalar fconst supported: {}", ty);

    // In the future, we may want to generate constant pool entries for these constants, but for
    // now use an `iconst` and a bit cast.
    let mut pos = FuncCursor::new(func).at_inst(inst);
    pos.use_srcloc(inst);
    let ival = match pos.func.dfg[inst] {
        ir::InstructionData::UnaryIeee32 {
            opcode: ir::Opcode::F32const,
            imm,
        } => pos.ins().iconst(ir::types::I32, i64::from(imm.bits())),
        ir::InstructionData::UnaryIeee64 {
            opcode: ir::Opcode::F64const,
            imm,
        } => pos.ins().iconst(ir::types::I64, imm.bits() as i64),
        _ => panic!("Expected fconst: {}", pos.func.dfg.display_inst(inst, None)),
    };
    pos.func.dfg.replace(inst).bitcast(ty, ival);
}

/// Expand illegal `stack_load` instructions.
fn expand_stack_load(
    inst: ir::Inst,
    func: &mut ir::Function,
    _cfg: &mut ControlFlowGraph,
    isa: &dyn TargetIsa,
) {
    let ty = func.dfg.value_type(func.dfg.first_result(inst));
    let addr_ty = isa.pointer_type();

    let mut pos = FuncCursor::new(func).at_inst(inst);
    pos.use_srcloc(inst);

    let (stack_slot, offset) = match pos.func.dfg[inst] {
        ir::InstructionData::StackLoad {
            opcode: _opcode,
            stack_slot,
            offset,
        } => (stack_slot, offset),
        _ => panic!(
            "Expected stack_load: {}",
            pos.func.dfg.display_inst(inst, None)
        ),
    };

    let addr = pos.ins().stack_addr(addr_ty, stack_slot, offset);

    // Stack slots are required to be accessible and aligned.
    let mflags = MemFlags::trusted();
    pos.func.dfg.replace(inst).load(ty, mflags, addr, 0);
}

/// Expand illegal `stack_store` instructions.
fn expand_stack_store(
    inst: ir::Inst,
    func: &mut ir::Function,
    _cfg: &mut ControlFlowGraph,
    isa: &dyn TargetIsa,
) {
    let addr_ty = isa.pointer_type();

    let mut pos = FuncCursor::new(func).at_inst(inst);
    pos.use_srcloc(inst);

    let (val, stack_slot, offset) = match pos.func.dfg[inst] {
        ir::InstructionData::StackStore {
            opcode: _opcode,
            arg,
            stack_slot,
            offset,
        } => (arg, stack_slot, offset),
        _ => panic!(
            "Expected stack_store: {}",
            pos.func.dfg.display_inst(inst, None)
        ),
    };

    let addr = pos.ins().stack_addr(addr_ty, stack_slot, offset);

    let mut mflags = MemFlags::new();
    // Stack slots are required to be accessible and aligned.
    mflags.set_notrap();
    mflags.set_aligned();
    pos.func.dfg.replace(inst).store(mflags, val, addr, 0);
}

/// Split a load into two parts before `iconcat`ing the result together.
fn narrow_load(
    inst: ir::Inst,
    func: &mut ir::Function,
    _cfg: &mut ControlFlowGraph,
    _isa: &dyn TargetIsa,
) {
    let mut pos = FuncCursor::new(func).at_inst(inst);
    pos.use_srcloc(inst);

    let (ptr, offset, flags) = match pos.func.dfg[inst] {
        ir::InstructionData::Load {
            opcode: ir::Opcode::Load,
            arg,
            offset,
            flags,
        } => (arg, offset, flags),
        _ => panic!("Expected load: {}", pos.func.dfg.display_inst(inst, None)),
    };

    let res_ty = pos.func.dfg.ctrl_typevar(inst);
    let small_ty = res_ty.half_width().expect("Can't narrow load");

    let al = pos.ins().load(small_ty, flags, ptr, offset);
    let ah = pos.ins().load(
        small_ty,
        flags,
        ptr,
        offset.try_add_i64(8).expect("load offset overflow"),
    );
    pos.func.dfg.replace(inst).iconcat(al, ah);
}

/// Split a store into two parts after `isplit`ing the value.
fn narrow_store(
    inst: ir::Inst,
    func: &mut ir::Function,
    _cfg: &mut ControlFlowGraph,
    _isa: &dyn TargetIsa,
) {
    let mut pos = FuncCursor::new(func).at_inst(inst);
    pos.use_srcloc(inst);

    let (val, ptr, offset, flags) = match pos.func.dfg[inst] {
        ir::InstructionData::Store {
            opcode: ir::Opcode::Store,
            args,
            offset,
            flags,
        } => (args[0], args[1], offset, flags),
        _ => panic!("Expected store: {}", pos.func.dfg.display_inst(inst, None)),
    };

    let (al, ah) = pos.ins().isplit(val);
    pos.ins().store(flags, al, ptr, offset);
    pos.ins().store(
        flags,
        ah,
        ptr,
        offset.try_add_i64(8).expect("store offset overflow"),
    );
    pos.remove_inst();
}
