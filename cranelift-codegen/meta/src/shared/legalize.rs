use crate::cdsl::ast::{var, ExprBuilder, Literal};
use crate::cdsl::instructions::{Instruction, InstructionGroup};
use crate::cdsl::xform::{TransformGroupBuilder, TransformGroups};

use crate::shared::immediates::Immediates;
use crate::shared::types::Float::{F32, F64};
use crate::shared::types::Int::{I128, I16, I32, I64, I8};

pub(crate) fn define(insts: &InstructionGroup, imm: &Immediates) -> TransformGroups {
    let mut narrow = TransformGroupBuilder::new(
        "narrow",
        r#"
        Legalize instructions by narrowing.

        The transformations in the 'narrow' group work by expressing
        instructions in terms of smaller types. Operations on vector types are
        expressed in terms of vector types with fewer lanes, and integer
        operations are expressed in terms of smaller integer types.
    "#,
    );

    let mut widen = TransformGroupBuilder::new(
        "widen",
        r#"
        Legalize instructions by widening.

        The transformations in the 'widen' group work by expressing
        instructions in terms of larger types.
    "#,
    );

    let mut expand = TransformGroupBuilder::new(
        "expand",
        r#"
        Legalize instructions by expansion.

        Rewrite instructions in terms of other instructions, generally
        operating on the same types as the original instructions.
    "#,
    );

    // List of instructions.
    let band = insts.by_name("band");
    let band_imm = insts.by_name("band_imm");
    let band_not = insts.by_name("band_not");
    let bint = insts.by_name("bint");
    let bitrev = insts.by_name("bitrev");
    let bnot = insts.by_name("bnot");
    let bor = insts.by_name("bor");
    let bor_imm = insts.by_name("bor_imm");
    let bor_not = insts.by_name("bor_not");
    let brnz = insts.by_name("brnz");
    let brz = insts.by_name("brz");
    let br_icmp = insts.by_name("br_icmp");
    let br_table = insts.by_name("br_table");
    let bxor = insts.by_name("bxor");
    let bxor_imm = insts.by_name("bxor_imm");
    let bxor_not = insts.by_name("bxor_not");
    let cls = insts.by_name("cls");
    let clz = insts.by_name("clz");
    let ctz = insts.by_name("ctz");
    let fabs = insts.by_name("fabs");
    let f32const = insts.by_name("f32const");
    let f64const = insts.by_name("f64const");
    let fcopysign = insts.by_name("fcopysign");
    let fcvt_from_sint = insts.by_name("fcvt_from_sint");
    let fcvt_from_uint = insts.by_name("fcvt_from_uint");
    let fneg = insts.by_name("fneg");
    let iadd = insts.by_name("iadd");
    let iadd_cin = insts.by_name("iadd_cin");
    let iadd_cout = insts.by_name("iadd_cout");
    let iadd_imm = insts.by_name("iadd_imm");
    let icmp = insts.by_name("icmp");
    let icmp_imm = insts.by_name("icmp_imm");
    let iconcat = insts.by_name("iconcat");
    let iconst = insts.by_name("iconst");
    let ifcmp = insts.by_name("ifcmp");
    let ifcmp_imm = insts.by_name("ifcmp_imm");
    let imul = insts.by_name("imul");
    let imul_imm = insts.by_name("imul_imm");
    let ireduce = insts.by_name("ireduce");
    let irsub_imm = insts.by_name("irsub_imm");
    let ishl = insts.by_name("ishl");
    let ishl_imm = insts.by_name("ishl_imm");
    let isplit = insts.by_name("isplit");
    let istore8 = insts.by_name("istore8");
    let istore16 = insts.by_name("istore16");
    let isub = insts.by_name("isub");
    let isub_bin = insts.by_name("isub_bin");
    let isub_bout = insts.by_name("isub_bout");
    let load = insts.by_name("load");
    let popcnt = insts.by_name("popcnt");
    let rotl = insts.by_name("rotl");
    let rotl_imm = insts.by_name("rotl_imm");
    let rotr = insts.by_name("rotr");
    let rotr_imm = insts.by_name("rotr_imm");
    let sdiv = insts.by_name("sdiv");
    let sdiv_imm = insts.by_name("sdiv_imm");
    let select = insts.by_name("select");
    let sextend = insts.by_name("sextend");
    let sshr = insts.by_name("sshr");
    let sshr_imm = insts.by_name("sshr_imm");
    let srem = insts.by_name("srem");
    let srem_imm = insts.by_name("srem_imm");
    let store = insts.by_name("store");
    let udiv = insts.by_name("udiv");
    let udiv_imm = insts.by_name("udiv_imm");
    let uextend = insts.by_name("uextend");
    let uload8 = insts.by_name("uload8");
    let uload16 = insts.by_name("uload16");
    let ushr = insts.by_name("ushr");
    let ushr_imm = insts.by_name("ushr_imm");
    let urem = insts.by_name("urem");
    let urem_imm = insts.by_name("urem_imm");
    let trapif = insts.by_name("trapif");
    let trapnz = insts.by_name("trapnz");
    let trapz = insts.by_name("trapz");

    // Custom expansions for memory objects.
    expand.custom_legalize(insts.by_name("global_value"), "expand_global_value");
    expand.custom_legalize(insts.by_name("heap_addr"), "expand_heap_addr");
    expand.custom_legalize(insts.by_name("table_addr"), "expand_table_addr");

    // Custom expansions for calls.
    expand.custom_legalize(insts.by_name("call"), "expand_call");

    // Custom expansions that need to change the CFG.
    // TODO: Add sufficient XForm syntax that we don't need to hand-code these.
    expand.custom_legalize(trapz, "expand_cond_trap");
    expand.custom_legalize(trapnz, "expand_cond_trap");
    expand.custom_legalize(br_table, "expand_br_table");
    expand.custom_legalize(select, "expand_select");

    // Custom expansions for floating point constants.
    // These expansions require bit-casting or creating constant pool entries.
    expand.custom_legalize(f32const, "expand_fconst");
    expand.custom_legalize(f64const, "expand_fconst");

    // Custom expansions for stack memory accesses.
    expand.custom_legalize(insts.by_name("stack_load"), "expand_stack_load");
    expand.custom_legalize(insts.by_name("stack_store"), "expand_stack_store");

    // List of variables to reuse in patterns.
    let x = var("x");
    let y = var("y");
    let z = var("z");
    let a = var("a");
    let a1 = var("a1");
    let a2 = var("a2");
    let a3 = var("a3");
    let a4 = var("a4");
    let b = var("b");
    let b1 = var("b1");
    let b2 = var("b2");
    let b3 = var("b3");
    let b4 = var("b4");
    let c = var("c");
    let c1 = var("c1");
    let c2 = var("c2");
    let c3 = var("c3");
    let c4 = var("c4");
    let d = var("d");
    let d1 = var("d1");
    let d2 = var("d2");
    let d3 = var("d3");
    let d4 = var("d4");
    let e = var("e");
    let e1 = var("e1");
    let e2 = var("e2");
    let e3 = var("e3");
    let e4 = var("e4");
    let f = var("f");
    let f1 = var("f1");
    let f2 = var("f2");
    let xl = var("xl");
    let xh = var("xh");
    let yl = var("yl");
    let yh = var("yh");
    let al = var("al");
    let ah = var("ah");
    let cc = var("cc");
    let ebb = var("ebb");
    let ptr = var("ptr");
    let flags = var("flags");
    let offset = var("off");
    let vararg = var("vararg");

    narrow.custom_legalize(load, "narrow_load");
    narrow.custom_legalize(store, "narrow_store");

    narrow.legalize(
        def!(a = iadd(x, y)),
        vec![
            def!((xl, xh) = isplit(x)),
            def!((yl, yh) = isplit(y)),
            def!((al, c) = iadd_cout(xl, yl)),
            def!(ah = iadd_cin(xh, yh, c)),
            def!(a = iconcat(al, ah)),
        ],
    );

    narrow.legalize(
        def!(a = isub(x, y)),
        vec![
            def!((xl, xh) = isplit(x)),
            def!((yl, yh) = isplit(y)),
            def!((al, b) = isub_bout(xl, yl)),
            def!(ah = isub_bin(xh, yh, b)),
            def!(a = iconcat(al, ah)),
        ],
    );

    for &bin_op in &[band, bor, bxor, band_not, bor_not, bxor_not] {
        narrow.legalize(
            def!(a = bin_op(x, y)),
            vec![
                def!((xl, xh) = isplit(x)),
                def!((yl, yh) = isplit(y)),
                def!(al = bin_op(xl, yl)),
                def!(ah = bin_op(xh, yh)),
                def!(a = iconcat(al, ah)),
            ],
        );
    }

    narrow.legalize(
        def!(a = bnot(x)),
        vec![
            def!((xl, xh) = isplit(x)),
            def!(al = bnot(xl)),
            def!(ah = bnot(xh)),
            def!(a = iconcat(al, ah)),
        ],
    );

    narrow.legalize(
        def!(a = select(c, x, y)),
        vec![
            def!((xl, xh) = isplit(x)),
            def!((yl, yh) = isplit(y)),
            def!(al = select(c, xl, yl)),
            def!(ah = select(c, xh, yh)),
            def!(a = iconcat(al, ah)),
        ],
    );

    narrow.legalize(
        def!(brz.I128(x, ebb, vararg)),
        vec![
            def!((xl, xh) = isplit(x)),
            def!(
                a = icmp_imm(
                    Literal::enumerator_for(&imm.intcc, "eq"),
                    xl,
                    Literal::constant(&imm.imm64, 0)
                )
            ),
            def!(
                b = icmp_imm(
                    Literal::enumerator_for(&imm.intcc, "eq"),
                    xh,
                    Literal::constant(&imm.imm64, 0)
                )
            ),
            def!(c = band(a, b)),
            def!(brz(c, ebb, vararg)),
        ],
    );

    narrow.legalize(
        def!(brnz.I128(x, ebb, vararg)),
        vec![
            def!((xl, xh) = isplit(x)),
            def!(brnz(xl, ebb, vararg)),
            def!(brnz(xh, ebb, vararg)),
        ],
    );

    // Widen instructions with one input operand.
    for &op in &[bnot, popcnt] {
        for &int_ty in &[I8, I16] {
            widen.legalize(
                def!(a = op.int_ty(b)),
                vec![
                    def!(x = uextend.I32(b)),
                    def!(z = op.I32(x)),
                    def!(a = ireduce.int_ty(z)),
                ],
            );
        }
    }

    // Widen instructions with two input operands.
    let mut widen_two_arg = |signed: bool, op: &Instruction| {
        for &int_ty in &[I8, I16] {
            let sign_ext_op = if signed { sextend } else { uextend };
            widen.legalize(
                def!(a = op.int_ty(b, c)),
                vec![
                    def!(x = sign_ext_op.I32(b)),
                    def!(y = sign_ext_op.I32(c)),
                    def!(z = op.I32(x, y)),
                    def!(a = ireduce.int_ty(z)),
                ],
            );
        }
    };

    for bin_op in &[
        iadd, isub, imul, udiv, urem, band, bor, bxor, band_not, bor_not, bxor_not,
    ] {
        widen_two_arg(false, bin_op);
    }
    for bin_op in &[sdiv, srem] {
        widen_two_arg(true, bin_op);
    }

    // Widen instructions using immediate operands.
    let mut widen_imm = |signed: bool, op: &Instruction| {
        for &int_ty in &[I8, I16] {
            let sign_ext_op = if signed { sextend } else { uextend };
            widen.legalize(
                def!(a = op.int_ty(b, c)),
                vec![
                    def!(x = sign_ext_op.I32(b)),
                    def!(z = op.I32(x, c)),
                    def!(a = ireduce.int_ty(z)),
                ],
            );
        }
    };

    for bin_op in &[
        iadd_imm, imul_imm, udiv_imm, urem_imm, band_imm, bor_imm, bxor_imm, irsub_imm,
    ] {
        widen_imm(false, bin_op);
    }
    for bin_op in &[sdiv_imm, srem_imm] {
        widen_imm(true, bin_op);
    }

    for &(int_ty, num) in &[(I8, 24), (I16, 16)] {
        let imm = Literal::constant(&imm.imm64, -num);

        widen.legalize(
            def!(a = clz.int_ty(b)),
            vec![
                def!(c = uextend.I32(b)),
                def!(d = clz.I32(c)),
                def!(e = iadd_imm(d, imm)),
                def!(a = ireduce.int_ty(e)),
            ],
        );

        widen.legalize(
            def!(a = cls.int_ty(b)),
            vec![
                def!(c = sextend.I32(b)),
                def!(d = cls.I32(c)),
                def!(e = iadd_imm(d, imm)),
                def!(a = ireduce.int_ty(e)),
            ],
        );
    }

    for &(int_ty, num) in &[(I8, 1 << 8), (I16, 1 << 16)] {
        let num = Literal::constant(&imm.imm64, num);
        widen.legalize(
            def!(a = ctz.int_ty(b)),
            vec![
                def!(c = uextend.I32(b)),
                // When `b` is zero, returns the size of x in bits.
                def!(d = bor_imm(c, num)),
                def!(e = ctz.I32(d)),
                def!(a = ireduce.int_ty(e)),
            ],
        );
    }

    // iconst
    for &int_ty in &[I8, I16] {
        widen.legalize(
            def!(a = iconst.int_ty(b)),
            vec![def!(c = iconst.I32(b)), def!(a = ireduce.int_ty(c))],
        );
    }

    for &extend_op in &[uextend, sextend] {
        // The sign extension operators have two typevars: the result has one and controls the
        // instruction, then the input has one.
        let bound = extend_op.bind(I16).bind(I8);
        widen.legalize(
            def!(a = bound(b)),
            vec![def!(c = extend_op.I32(b)), def!(a = ireduce(c))],
        );
    }

    widen.legalize(
        def!(store.I8(flags, a, ptr, offset)),
        vec![
            def!(b = uextend.I32(a)),
            def!(istore8(flags, b, ptr, offset)),
        ],
    );

    widen.legalize(
        def!(store.I16(flags, a, ptr, offset)),
        vec![
            def!(b = uextend.I32(a)),
            def!(istore16(flags, b, ptr, offset)),
        ],
    );

    widen.legalize(
        def!(a = load.I8(flags, ptr, offset)),
        vec![
            def!(b = uload8.I32(flags, ptr, offset)),
            def!(a = ireduce(b)),
        ],
    );

    widen.legalize(
        def!(a = load.I16(flags, ptr, offset)),
        vec![
            def!(b = uload16.I32(flags, ptr, offset)),
            def!(a = ireduce(b)),
        ],
    );

    for &int_ty in &[I8, I16] {
        widen.legalize(
            def!(br_table.int_ty(x, y, z)),
            vec![def!(b = uextend.I32(x)), def!(br_table(b, y, z))],
        );
    }

    for &int_ty in &[I8, I16] {
        widen.legalize(
            def!(a = bint.int_ty(b)),
            vec![def!(x = bint.I32(b)), def!(a = ireduce.int_ty(x))],
        );
    }

    for &int_ty in &[I8, I16] {
        for &op in &[ishl, ishl_imm, ushr, ushr_imm] {
            widen.legalize(
                def!(a = op.int_ty(b, c)),
                vec![
                    def!(x = uextend.I32(b)),
                    def!(z = op.I32(x, c)),
                    def!(a = ireduce.int_ty(z)),
                ],
            );
        }

        for &op in &[sshr, sshr_imm] {
            widen.legalize(
                def!(a = op.int_ty(b, c)),
                vec![
                    def!(x = sextend.I32(b)),
                    def!(z = op.I32(x, c)),
                    def!(a = ireduce.int_ty(z)),
                ],
            );
        }

        for cc in &["eq", "ne", "ugt", "ult", "uge", "ule"] {
            let w_cc = Literal::enumerator_for(&imm.intcc, cc);
            widen.legalize(
                def!(a = icmp_imm.int_ty(w_cc, b, c)),
                vec![def!(x = uextend.I32(b)), def!(a = icmp_imm(w_cc, x, c))],
            );
            widen.legalize(
                def!(a = icmp.int_ty(w_cc, b, c)),
                vec![
                    def!(x = uextend.I32(b)),
                    def!(y = uextend.I32(c)),
                    def!(a = icmp.I32(w_cc, x, y)),
                ],
            );
        }

        for cc in &["sgt", "slt", "sge", "sle"] {
            let w_cc = Literal::enumerator_for(&imm.intcc, cc);
            widen.legalize(
                def!(a = icmp_imm.int_ty(w_cc, b, c)),
                vec![def!(x = sextend.I32(b)), def!(a = icmp_imm(w_cc, x, c))],
            );

            widen.legalize(
                def!(a = icmp.int_ty(w_cc, b, c)),
                vec![
                    def!(x = sextend.I32(b)),
                    def!(y = sextend.I32(c)),
                    def!(a = icmp(w_cc, x, y)),
                ],
            );
        }
    }

    // Expansions for fcvt_from_{u,s}int for smaller integer types.
    // These use expand and not widen because the controlling type variable for
    // these instructions are f32/f64, which are legalized as part of the expand
    // group.
    for &dest_ty in &[F32, F64] {
        for &src_ty in &[I8, I16] {
            let bound_inst = fcvt_from_uint.bind(dest_ty).bind(src_ty);
            expand.legalize(
                def!(a = bound_inst(b)),
                vec![
                    def!(x = uextend.I32(b)),
                    def!(a = fcvt_from_uint.dest_ty(x)),
                ],
            );

            let bound_inst = fcvt_from_sint.bind(dest_ty).bind(src_ty);
            expand.legalize(
                def!(a = bound_inst(b)),
                vec![
                    def!(x = sextend.I32(b)),
                    def!(a = fcvt_from_sint.dest_ty(x)),
                ],
            );
        }
    }

    // Expansions for immediate operands that are out of range.
    for &(inst_imm, inst) in &[
        (iadd_imm, iadd),
        (imul_imm, imul),
        (sdiv_imm, sdiv),
        (udiv_imm, udiv),
        (srem_imm, srem),
        (urem_imm, urem),
        (band_imm, band),
        (bor_imm, bor),
        (bxor_imm, bxor),
        (ifcmp_imm, ifcmp),
    ] {
        expand.legalize(
            def!(a = inst_imm(x, y)),
            vec![def!(a1 = iconst(y)), def!(a = inst(x, a1))],
        );
    }

    expand.legalize(
        def!(a = irsub_imm(y, x)),
        vec![def!(a1 = iconst(x)), def!(a = isub(a1, y))],
    );

    // Rotates and shifts.
    for &(inst_imm, inst) in &[
        (rotl_imm, rotl),
        (rotr_imm, rotr),
        (ishl_imm, ishl),
        (sshr_imm, sshr),
        (ushr_imm, ushr),
    ] {
        expand.legalize(
            def!(a = inst_imm(x, y)),
            vec![def!(a1 = iconst.I32(y)), def!(a = inst(x, a1))],
        );
    }

    expand.legalize(
        def!(a = icmp_imm(cc, x, y)),
        vec![def!(a1 = iconst(y)), def!(a = icmp(cc, x, a1))],
    );

    //# Expansions for *_not variants of bitwise ops.
    for &(inst_not, inst) in &[(band_not, band), (bor_not, bor), (bxor_not, bxor)] {
        expand.legalize(
            def!(a = inst_not(x, y)),
            vec![def!(a1 = bnot(y)), def!(a = inst(x, a1))],
        );
    }

    //# Expand bnot using xor.
    let minus_one = Literal::constant(&imm.imm64, -1);
    expand.legalize(
        def!(a = bnot(x)),
        vec![def!(y = iconst(minus_one)), def!(a = bxor(x, y))],
    );

    //# Expand bitrev
    //# Adapted from Stack Overflow.
    //# https://stackoverflow.com/questions/746171/most-efficient-algorithm-for-bit-reversal-from-msb-lsb-to-lsb-msb-in-c
    let imm64_1 = Literal::constant(&imm.imm64, 1);
    let imm64_2 = Literal::constant(&imm.imm64, 2);
    let imm64_4 = Literal::constant(&imm.imm64, 4);

    widen.legalize(
        def!(a = bitrev.I8(x)),
        vec![
            def!(a1 = band_imm(x, Literal::constant(&imm.imm64, 0xaa))),
            def!(a2 = ushr_imm(a1, imm64_1)),
            def!(a3 = band_imm(x, Literal::constant(&imm.imm64, 0x55))),
            def!(a4 = ishl_imm(a3, imm64_1)),
            def!(b = bor(a2, a4)),
            def!(b1 = band_imm(b, Literal::constant(&imm.imm64, 0xcc))),
            def!(b2 = ushr_imm(b1, imm64_2)),
            def!(b3 = band_imm(b, Literal::constant(&imm.imm64, 0x33))),
            def!(b4 = ishl_imm(b3, imm64_2)),
            def!(c = bor(b2, b4)),
            def!(c1 = band_imm(c, Literal::constant(&imm.imm64, 0xf0))),
            def!(c2 = ushr_imm(c1, imm64_4)),
            def!(c3 = band_imm(c, Literal::constant(&imm.imm64, 0x0f))),
            def!(c4 = ishl_imm(c3, imm64_4)),
            def!(a = bor(c2, c4)),
        ],
    );

    let imm64_8 = Literal::constant(&imm.imm64, 8);

    widen.legalize(
        def!(a = bitrev.I16(x)),
        vec![
            def!(a1 = band_imm(x, Literal::constant(&imm.imm64, 0xaaaa))),
            def!(a2 = ushr_imm(a1, imm64_1)),
            def!(a3 = band_imm(x, Literal::constant(&imm.imm64, 0x5555))),
            def!(a4 = ishl_imm(a3, imm64_1)),
            def!(b = bor(a2, a4)),
            def!(b1 = band_imm(b, Literal::constant(&imm.imm64, 0xcccc))),
            def!(b2 = ushr_imm(b1, imm64_2)),
            def!(b3 = band_imm(b, Literal::constant(&imm.imm64, 0x3333))),
            def!(b4 = ishl_imm(b3, imm64_2)),
            def!(c = bor(b2, b4)),
            def!(c1 = band_imm(c, Literal::constant(&imm.imm64, 0xf0f0))),
            def!(c2 = ushr_imm(c1, imm64_4)),
            def!(c3 = band_imm(c, Literal::constant(&imm.imm64, 0x0f0f))),
            def!(c4 = ishl_imm(c3, imm64_4)),
            def!(d = bor(c2, c4)),
            def!(d1 = band_imm(d, Literal::constant(&imm.imm64, 0xff00))),
            def!(d2 = ushr_imm(d1, imm64_8)),
            def!(d3 = band_imm(d, Literal::constant(&imm.imm64, 0x00ff))),
            def!(d4 = ishl_imm(d3, imm64_8)),
            def!(a = bor(d2, d4)),
        ],
    );

    let imm64_16 = Literal::constant(&imm.imm64, 16);

    expand.legalize(
        def!(a = bitrev.I32(x)),
        vec![
            def!(a1 = band_imm(x, Literal::constant(&imm.imm64, 0xaaaaaaaa))),
            def!(a2 = ushr_imm(a1, imm64_1)),
            def!(a3 = band_imm(x, Literal::constant(&imm.imm64, 0x55555555))),
            def!(a4 = ishl_imm(a3, imm64_1)),
            def!(b = bor(a2, a4)),
            def!(b1 = band_imm(b, Literal::constant(&imm.imm64, 0xcccccccc))),
            def!(b2 = ushr_imm(b1, imm64_2)),
            def!(b3 = band_imm(b, Literal::constant(&imm.imm64, 0x33333333))),
            def!(b4 = ishl_imm(b3, imm64_2)),
            def!(c = bor(b2, b4)),
            def!(c1 = band_imm(c, Literal::constant(&imm.imm64, 0xf0f0f0f0))),
            def!(c2 = ushr_imm(c1, imm64_4)),
            def!(c3 = band_imm(c, Literal::constant(&imm.imm64, 0x0f0f0f0f))),
            def!(c4 = ishl_imm(c3, imm64_4)),
            def!(d = bor(c2, c4)),
            def!(d1 = band_imm(d, Literal::constant(&imm.imm64, 0xff00ff00))),
            def!(d2 = ushr_imm(d1, imm64_8)),
            def!(d3 = band_imm(d, Literal::constant(&imm.imm64, 0x00ff00ff))),
            def!(d4 = ishl_imm(d3, imm64_8)),
            def!(e = bor(d2, d4)),
            def!(e1 = ushr_imm(e, imm64_16)),
            def!(e2 = ishl_imm(e, imm64_16)),
            def!(a = bor(e1, e2)),
        ],
    );

    #[allow(overflowing_literals)]
    let imm64_0xaaaaaaaaaaaaaaaa = Literal::constant(&imm.imm64, 0xaaaaaaaaaaaaaaaa);
    let imm64_0x5555555555555555 = Literal::constant(&imm.imm64, 0x5555555555555555);
    #[allow(overflowing_literals)]
    let imm64_0xcccccccccccccccc = Literal::constant(&imm.imm64, 0xcccccccccccccccc);
    let imm64_0x3333333333333333 = Literal::constant(&imm.imm64, 0x3333333333333333);
    #[allow(overflowing_literals)]
    let imm64_0xf0f0f0f0f0f0f0f0 = Literal::constant(&imm.imm64, 0xf0f0f0f0f0f0f0f0);
    let imm64_0x0f0f0f0f0f0f0f0f = Literal::constant(&imm.imm64, 0x0f0f0f0f0f0f0f0f);
    #[allow(overflowing_literals)]
    let imm64_0xff00ff00ff00ff00 = Literal::constant(&imm.imm64, 0xff00ff00ff00ff00);
    let imm64_0x00ff00ff00ff00ff = Literal::constant(&imm.imm64, 0x00ff00ff00ff00ff);
    #[allow(overflowing_literals)]
    let imm64_0xffff0000ffff0000 = Literal::constant(&imm.imm64, 0xffff0000ffff0000);
    let imm64_0x0000ffff0000ffff = Literal::constant(&imm.imm64, 0x0000ffff0000ffff);
    let imm64_32 = Literal::constant(&imm.imm64, 32);

    expand.legalize(
        def!(a = bitrev.I64(x)),
        vec![
            def!(a1 = band_imm(x, imm64_0xaaaaaaaaaaaaaaaa)),
            def!(a2 = ushr_imm(a1, imm64_1)),
            def!(a3 = band_imm(x, imm64_0x5555555555555555)),
            def!(a4 = ishl_imm(a3, imm64_1)),
            def!(b = bor(a2, a4)),
            def!(b1 = band_imm(b, imm64_0xcccccccccccccccc)),
            def!(b2 = ushr_imm(b1, imm64_2)),
            def!(b3 = band_imm(b, imm64_0x3333333333333333)),
            def!(b4 = ishl_imm(b3, imm64_2)),
            def!(c = bor(b2, b4)),
            def!(c1 = band_imm(c, imm64_0xf0f0f0f0f0f0f0f0)),
            def!(c2 = ushr_imm(c1, imm64_4)),
            def!(c3 = band_imm(c, imm64_0x0f0f0f0f0f0f0f0f)),
            def!(c4 = ishl_imm(c3, imm64_4)),
            def!(d = bor(c2, c4)),
            def!(d1 = band_imm(d, imm64_0xff00ff00ff00ff00)),
            def!(d2 = ushr_imm(d1, imm64_8)),
            def!(d3 = band_imm(d, imm64_0x00ff00ff00ff00ff)),
            def!(d4 = ishl_imm(d3, imm64_8)),
            def!(e = bor(d2, d4)),
            def!(e1 = band_imm(e, imm64_0xffff0000ffff0000)),
            def!(e2 = ushr_imm(e1, imm64_16)),
            def!(e3 = band_imm(e, imm64_0x0000ffff0000ffff)),
            def!(e4 = ishl_imm(e3, imm64_16)),
            def!(f = bor(e2, e4)),
            def!(f1 = ushr_imm(f, imm64_32)),
            def!(f2 = ishl_imm(f, imm64_32)),
            def!(a = bor(f1, f2)),
        ],
    );

    // Floating-point sign manipulations.
    for &(ty, const_inst, minus_zero) in &[
        (F32, f32const, &Literal::bits(&imm.ieee32, 0x80000000)),
        (
            F64,
            f64const,
            &Literal::bits(&imm.ieee64, 0x8000000000000000),
        ),
    ] {
        expand.legalize(
            def!(a = fabs.ty(x)),
            vec![def!(b = const_inst(minus_zero)), def!(a = band_not(x, b))],
        );

        expand.legalize(
            def!(a = fneg.ty(x)),
            vec![def!(b = const_inst(minus_zero)), def!(a = bxor(x, b))],
        );

        expand.legalize(
            def!(a = fcopysign.ty(x, y)),
            vec![
                def!(b = const_inst(minus_zero)),
                def!(a1 = band_not(x, b)),
                def!(a2 = band(y, b)),
                def!(a = bor(a1, a2)),
            ],
        );
    }

    expand.custom_legalize(br_icmp, "expand_br_icmp");

    let mut groups = TransformGroups::new();

    narrow.build_and_add_to(&mut groups);
    let expand_id = expand.build_and_add_to(&mut groups);

    // Expansions using CPU flags.
    let mut expand_flags = TransformGroupBuilder::new(
        "expand_flags",
        r#"
        Instruction expansions for architectures with flags.

        Expand some instructions using CPU flags, then fall back to the normal
        expansions. Not all architectures support CPU flags, so these patterns
        are kept separate.
    "#,
    )
    .chain_with(expand_id);

    let imm64_0 = Literal::constant(&imm.imm64, 0);
    let intcc_ne = Literal::enumerator_for(&imm.intcc, "ne");
    let intcc_eq = Literal::enumerator_for(&imm.intcc, "eq");

    expand_flags.legalize(
        def!(trapnz(x, c)),
        vec![
            def!(a = ifcmp_imm(x, imm64_0)),
            def!(trapif(intcc_ne, a, c)),
        ],
    );

    expand_flags.legalize(
        def!(trapz(x, c)),
        vec![
            def!(a = ifcmp_imm(x, imm64_0)),
            def!(trapif(intcc_eq, a, c)),
        ],
    );

    expand_flags.build_and_add_to(&mut groups);

    // TODO The order of declarations unfortunately matters to be compatible with the Python code.
    // When it's all migrated, we can put this next to the narrow/expand build_and_add_to calls
    // above.
    widen.build_and_add_to(&mut groups);

    groups
}
