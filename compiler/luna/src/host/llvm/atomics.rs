//! Atomic memory operations, fences, and runtime panic/abort intrinsic lowering for LLVM IR.
use super::*;

const ALL_ORDERS: &[(u8, &str)] = &[
    (0, "monotonic"),
    (1, "acquire"),
    (2, "release"),
    (3, "acq_rel"),
    (4, "seq_cst"),
];

const LOAD_ORDERS: &[(u8, &str)] = &[(0, "monotonic"), (1, "acquire"), (4, "seq_cst")];

const STORE_ORDERS: &[(u8, &str)] = &[(0, "monotonic"), (2, "release"), (4, "seq_cst")];

const FENCE_ORDERS: &[(u8, &str)] = &[
    (1, "acquire"),
    (2, "release"),
    (3, "acq_rel"),
    (4, "seq_cst"),
];

const CMPXCHG_PAIRS: &[(u16, &str, &str)] = &[
    ((0 << 8) | 0, "monotonic", "monotonic"),
    ((1 << 8) | 0, "acquire", "monotonic"),
    ((1 << 8) | 1, "acquire", "acquire"),
    ((2 << 8) | 0, "release", "monotonic"),
    ((3 << 8) | 0, "acq_rel", "monotonic"),
    ((3 << 8) | 1, "acq_rel", "acquire"),
    ((4 << 8) | 0, "seq_cst", "monotonic"),
    ((4 << 8) | 1, "seq_cst", "acquire"),
    ((4 << 8) | 4, "seq_cst", "seq_cst"),
];

#[derive(Clone)]
struct AtomicCarrier {
    user_ty: Ty,
    llvm_ty: String,
    align: u32,
    is_bool: bool,
    is_signed: bool,
}

fn parse_const_u8(value: &Value) -> Option<u8> {
    if value.ty != Ty::Int(8, false) {
        return None;
    }
    value.text.trim().parse::<u8>().ok()
}

impl Emitter<'_> {
    fn pointer_bytes(&self) -> u32 {
        self.graph
            .target
            .map_or(8, |target| (target.pointer_bits() / 8).max(1))
    }

    fn atomic_carrier(&self, ty: &Ty, allow_non_int: bool) -> Result<AtomicCarrier, String> {
        match ty {
            Ty::Int(bits @ (8 | 16 | 32 | 64), signed) => Ok(AtomicCarrier {
                user_ty: ty.clone(),
                llvm_ty: format!("i{bits}"),
                align: bits / 8,
                is_bool: false,
                is_signed: *signed,
            }),
            Ty::Bool if allow_non_int => Ok(AtomicCarrier {
                user_ty: Ty::Bool,
                llvm_ty: "i8".into(),
                align: 1,
                is_bool: true,
                is_signed: false,
            }),
            Ty::Pointer if allow_non_int => Ok(AtomicCarrier {
                user_ty: Ty::Pointer,
                llvm_ty: "ptr".into(),
                align: self.pointer_bytes(),
                is_bool: false,
                is_signed: false,
            }),
            _ => Err(format!(
                "host atomic operation does not support representation {}",
                ty.llvm()
            )),
        }
    }

    fn carrier_from_user(
        &mut self,
        carrier: &AtomicCarrier,
        value: &Value,
    ) -> Result<String, String> {
        if value.ty != carrier.user_ty {
            return Err("host atomic value representation mismatch".into());
        }
        if carrier.is_bool {
            let ext = self.instruction(Ty::Int(8, false), format!("zext i1 {} to i8", value.text));
            Ok(ext.text)
        } else {
            Ok(value.text.clone())
        }
    }

    fn carrier_to_user(&mut self, carrier: &AtomicCarrier, raw_name: &str) -> Value {
        if carrier.is_bool {
            self.instruction(Ty::Bool, format!("icmp ne i8 {raw_name}, 0"))
        } else {
            Value {
                ty: carrier.user_ty.clone(),
                text: raw_name.to_owned(),
            }
        }
    }

    fn dummy_value(&self, ty: &Ty) -> Value {
        match ty {
            Ty::Unit => Value::unit(),
            _ => Value {
                ty: ty.clone(),
                text: "poison".into(),
            },
        }
    }

    fn dispatch_atomic_order(
        &mut self,
        order: &Value,
        allowed: &[(u8, &str)],
        result_ty: &Ty,
        mut emit_arm: impl FnMut(&mut Self, &str) -> Result<Value, String>,
    ) -> Result<Value, String> {
        if order.ty != Ty::Int(8, false) {
            return Err("host atomic ordering requires u8 tag".into());
        }
        self.emitted_effects.has_trap_or_unwind = true;
        if let Some(tag) = parse_const_u8(order) {
            if let Some(&(_, llvm_order)) = allowed.iter().find(|&&(k, _)| k == tag) {
                return emit_arm(self, llvm_order);
            }
            self.emit_trap();
            self.line("unreachable");
            self.terminated = true;
            return Ok(self.dummy_value(result_ty));
        }

        let bad = self.fresh();
        let merge = self.fresh();
        let mut arm_labels = Vec::with_capacity(allowed.len());
        let mut switch_targets = String::new();
        for &(tag, _) in allowed {
            let arm = self.fresh();
            switch_targets.push_str(&format!(" i8 {tag}, label %{arm}"));
            arm_labels.push(arm);
        }
        self.line(format!(
            "switch i8 {}, label %{bad} [{switch_targets} ]",
            order.text
        ));
        self.terminated = true;

        self.label(&bad);
        self.emit_trap();
        self.line("unreachable");
        self.terminated = true;

        let mut incoming = Vec::with_capacity(allowed.len());
        for ((_, llvm_order), arm_label) in allowed.iter().zip(arm_labels) {
            self.label(&arm_label);
            let value = emit_arm(self, llvm_order)?;
            if value.ty != *result_ty {
                return Err("host atomic arm result representation mismatch".into());
            }
            let exit_label = self.fresh();
            self.branch(&exit_label);
            self.label(&exit_label);
            self.branch(&merge);
            incoming.push((value.text, exit_label));
        }

        self.label(&merge);
        if *result_ty == Ty::Unit {
            Ok(Value::unit())
        } else {
            let pairs = incoming
                .into_iter()
                .map(|(val, lbl)| format!("[ {val}, %{lbl} ]"))
                .collect::<Vec<_>>()
                .join(", ");
            Ok(self.instruction(
                result_ty.clone(),
                format!("phi {} {pairs}", result_ty.llvm()),
            ))
        }
    }

    fn dispatch_cmpxchg_orders(
        &mut self,
        succ: &Value,
        fail: &Value,
        mut emit_arm: impl FnMut(&mut Self, &str, &str) -> Result<Value, String>,
    ) -> Result<Value, String> {
        if succ.ty != Ty::Int(8, false) || fail.ty != Ty::Int(8, false) {
            return Err("host atomic cmpxchg ordering requires u8 tags".into());
        }
        self.emitted_effects.has_trap_or_unwind = true;
        if let (Some(s), Some(f)) = (parse_const_u8(succ), parse_const_u8(fail)) {
            let code = (u16::from(s) << 8) | u16::from(f);
            if let Some(&(_, succ_str, fail_str)) =
                CMPXCHG_PAIRS.iter().find(|&&(k, _, _)| k == code)
            {
                return emit_arm(self, succ_str, fail_str);
            }
            self.emit_trap();
            self.line("unreachable");
            self.terminated = true;
            return Ok(self.dummy_value(&Ty::Bool));
        }

        let succ_wide =
            self.instruction(Ty::Int(16, false), format!("zext i8 {} to i16", succ.text));
        let succ_shifted =
            self.instruction(Ty::Int(16, false), format!("shl i16 {}, 8", succ_wide.text));
        let fail_wide =
            self.instruction(Ty::Int(16, false), format!("zext i8 {} to i16", fail.text));
        let code = self.instruction(
            Ty::Int(16, false),
            format!("or i16 {}, {}", succ_shifted.text, fail_wide.text),
        );

        let bad = self.fresh();
        let merge = self.fresh();
        let mut arm_labels = Vec::with_capacity(CMPXCHG_PAIRS.len());
        let mut switch_targets = String::new();
        for &(pair_code, _, _) in CMPXCHG_PAIRS {
            let arm = self.fresh();
            switch_targets.push_str(&format!(" i16 {pair_code}, label %{arm}"));
            arm_labels.push(arm);
        }
        self.line(format!(
            "switch i16 {}, label %{bad} [{switch_targets} ]",
            code.text
        ));
        self.terminated = true;

        self.label(&bad);
        self.emit_trap();
        self.line("unreachable");
        self.terminated = true;

        let mut incoming = Vec::with_capacity(CMPXCHG_PAIRS.len());
        for ((_, succ_str, fail_str), arm_label) in CMPXCHG_PAIRS.iter().zip(arm_labels) {
            self.label(&arm_label);
            let ok = emit_arm(self, succ_str, fail_str)?;
            let exit_label = self.fresh();
            self.branch(&exit_label);
            self.label(&exit_label);
            self.branch(&merge);
            incoming.push((ok.text, exit_label));
        }

        self.label(&merge);
        let pairs = incoming
            .into_iter()
            .map(|(val, lbl)| format!("[ {val}, %{lbl} ]"))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(self.instruction(Ty::Bool, format!("phi i1 {pairs}")))
    }

    pub(super) fn atomic_intrinsic(
        &mut self,
        operation: nir::intrinsics::Operation,
        values: &[Value],
        result_ty: Ty,
    ) -> Result<Value, String> {
        use nir::intrinsics::Operation;
        match operation {
            Operation::AtomicFence | Operation::AtomicCompilerFence => {
                let [order] = values else {
                    return Err("host atomic fence requires 1 ordering operand".into());
                };
                if result_ty != Ty::Unit {
                    return Err("host atomic fence requires unit result".into());
                }
                let syncscope = if operation == Operation::AtomicCompilerFence {
                    "syncscope(\"singlethread\") "
                } else {
                    ""
                };
                self.dispatch_atomic_order(order, FENCE_ORDERS, &Ty::Unit, |this, ord| {
                    this.line(format!("fence {syncscope}{ord}"));
                    Ok(Value::unit())
                })
            }
            Operation::AtomicLoad => {
                let [ptr, order] = values else {
                    return Err("host atomic_load requires 2 operands".into());
                };
                if ptr.ty != Ty::Pointer {
                    return Err("host atomic_load requires pointer operand".into());
                }
                let carrier = self.atomic_carrier(&result_ty, true)?;
                self.dispatch_atomic_order(order, LOAD_ORDERS, &result_ty, |this, ord| {
                    let raw = this.fresh();
                    this.line(format!(
                        "%{raw} = load atomic {}, ptr {} {ord}, align {}",
                        carrier.llvm_ty, ptr.text, carrier.align
                    ));
                    Ok(this.carrier_to_user(&carrier, &format!("%{raw}")))
                })
            }
            Operation::AtomicStore => {
                let [ptr, val, order] = values else {
                    return Err("host atomic_store requires 3 operands".into());
                };
                if ptr.ty != Ty::Pointer || result_ty != Ty::Unit {
                    return Err("host atomic_store representation mismatch".into());
                }
                let carrier = self.atomic_carrier(&val.ty, true)?;
                let raw_val = self.carrier_from_user(&carrier, val)?;
                self.dispatch_atomic_order(order, STORE_ORDERS, &Ty::Unit, |this, ord| {
                    this.line(format!(
                        "store atomic {} {raw_val}, ptr {} {ord}, align {}",
                        carrier.llvm_ty, ptr.text, carrier.align
                    ));
                    Ok(Value::unit())
                })
            }
            Operation::AtomicSwap => {
                let [ptr, val, order] = values else {
                    return Err("host atomic_swap requires 3 operands".into());
                };
                if ptr.ty != Ty::Pointer || val.ty != result_ty {
                    return Err("host atomic_swap representation mismatch".into());
                }
                let carrier = self.atomic_carrier(&result_ty, true)?;
                let raw_val = self.carrier_from_user(&carrier, val)?;
                self.dispatch_atomic_order(order, ALL_ORDERS, &result_ty, |this, ord| {
                    let raw = this.fresh();
                    this.line(format!(
                        "%{raw} = atomicrmw xchg ptr {}, {} {raw_val} {ord}, align {}",
                        ptr.text, carrier.llvm_ty, carrier.align
                    ));
                    Ok(this.carrier_to_user(&carrier, &format!("%{raw}")))
                })
            }
            Operation::AtomicFetchAdd
            | Operation::AtomicFetchSub
            | Operation::AtomicFetchAnd
            | Operation::AtomicFetchOr
            | Operation::AtomicFetchXor
            | Operation::AtomicFetchMax
            | Operation::AtomicFetchMin => {
                let [ptr, val, order] = values else {
                    return Err("host atomic fetch-rmw requires 3 operands".into());
                };
                if ptr.ty != Ty::Pointer || val.ty != result_ty {
                    return Err("host atomic fetch-rmw representation mismatch".into());
                }
                let carrier = self.atomic_carrier(&result_ty, false)?;
                let rmw_op = match (operation, carrier.is_signed) {
                    (Operation::AtomicFetchAdd, _) => "add",
                    (Operation::AtomicFetchSub, _) => "sub",
                    (Operation::AtomicFetchAnd, _) => "and",
                    (Operation::AtomicFetchOr, _) => "or",
                    (Operation::AtomicFetchXor, _) => "xor",
                    (Operation::AtomicFetchMax, true) => "max",
                    (Operation::AtomicFetchMax, false) => "umax",
                    (Operation::AtomicFetchMin, true) => "min",
                    (Operation::AtomicFetchMin, false) => "umin",
                    _ => unreachable!(),
                };
                let raw_val = self.carrier_from_user(&carrier, val)?;
                self.dispatch_atomic_order(order, ALL_ORDERS, &result_ty, |this, ord| {
                    let raw = this.fresh();
                    this.line(format!(
                        "%{raw} = atomicrmw {rmw_op} ptr {}, {} {raw_val} {ord}, align {}",
                        ptr.text, carrier.llvm_ty, carrier.align
                    ));
                    Ok(this.carrier_to_user(&carrier, &format!("%{raw}")))
                })
            }
            Operation::AtomicCompareExchange | Operation::AtomicCompareExchangeWeak => {
                let [ptr, expected_ptr, desired, succ, fail] = values else {
                    return Err("host atomic_cmpxchg requires 5 operands".into());
                };
                if ptr.ty != Ty::Pointer || expected_ptr.ty != Ty::Pointer || result_ty != Ty::Bool
                {
                    return Err("host atomic_cmpxchg representation mismatch".into());
                }
                let carrier = self.atomic_carrier(&desired.ty, true)?;
                let des_raw = self.carrier_from_user(&carrier, desired)?;
                let exp_loaded = self.load(&Slot::new(
                    carrier.user_ty.clone(),
                    expected_ptr.text.clone(),
                    Some(if carrier.is_bool {
                        1
                    } else {
                        u64::from(carrier.align)
                    }),
                ));
                let exp_raw = self.carrier_from_user(&carrier, &exp_loaded)?;
                let weak_kw = if operation == Operation::AtomicCompareExchangeWeak {
                    "weak "
                } else {
                    ""
                };
                self.dispatch_cmpxchg_orders(succ, fail, |this, succ_str, fail_str| {
                    let pair = this.fresh();
                    this.line(format!(
                        "%{pair} = cmpxchg {weak_kw}ptr {}, {} {exp_raw}, {} {des_raw} {succ_str} {fail_str}, align {}",
                        ptr.text, carrier.llvm_ty, carrier.llvm_ty, carrier.align
                    ));
                    let old_raw = this.fresh();
                    this.line(format!(
                        "%{old_raw} = extractvalue {{ {}, i1 }} %{pair}, 0",
                        carrier.llvm_ty
                    ));
                    let ok = this.instruction(
                        Ty::Bool,
                        format!("extractvalue {{ {}, i1 }} %{pair}, 1", carrier.llvm_ty),
                    );
                    let done = this.fresh();
                    let update_expected = this.fresh();
                    this.line(format!(
                        "br i1 {}, label %{done}, label %{update_expected}",
                        ok.text
                    ));
                    this.label(&update_expected);
                    let old_user = this.carrier_to_user(&carrier, &format!("%{old_raw}"));
                    this.store(
                        &Slot::new(
                            carrier.user_ty.clone(),
                            expected_ptr.text.clone(),
                            Some(if carrier.is_bool {
                                1
                            } else {
                                u64::from(carrier.align)
                            }),
                        ),
                        &old_user,
                    )?;
                    this.branch(&done);
                    this.label(&done);
                    Ok(ok)
                })
            }
            _ => Err("unsupported host atomic intrinsic".into()),
        }
    }

    pub(super) fn control_runtime_intrinsic(
        &mut self,
        operation: nir::intrinsics::Operation,
        values: &[Value],
        source: nir::program::SourceRange,
    ) -> Result<Value, String> {
        use nir::intrinsics::Operation;
        match operation {
            Operation::RuntimeAbort => {
                if !values.is_empty() {
                    return Err("host runtime_abort takes 0 operands".into());
                }
                self.emit_trap();
                self.line("unreachable");
                self.terminated = true;
                Ok(Value::unit())
            }
            Operation::RuntimePanic => {
                let [message] = values else {
                    return Err("host runtime_panic requires 1 message operand".into());
                };
                if message.ty != Ty::String {
                    return Err("host runtime_panic requires str operand".into());
                }
                self.declarations
                    .insert("declare i32 @dprintf(i32, ptr, ...)".into());
                self.declarations.insert(
                    "@flurry_panic_format = private unnamed_addr constant [13 x i8] c\"panic: %.*s\\0A\\00\""
                        .into(),
                );
                let data = self.instruction(
                    Ty::Pointer,
                    format!("extractvalue {{ ptr, i64 }} {}, 0", message.text),
                );
                let len64 = self.instruction(
                    Ty::Int(64, false),
                    format!("extractvalue {{ ptr, i64 }} {}, 1", message.text),
                );
                let too_long =
                    self.instruction(Ty::Bool, format!("icmp ugt i64 {}, 2147483647", len64.text));
                let clamped = self.instruction(
                    Ty::Int(64, false),
                    format!(
                        "select i1 {}, i64 2147483647, i64 {}",
                        too_long.text, len64.text
                    ),
                );
                let len32 = self.instruction(
                    Ty::Int(32, true),
                    format!("trunc i64 {} to i32", clamped.text),
                );
                self.emitted_effects.has_external_or_indirect_call = true;
                self.emitted_effects.has_trap_or_unwind = true;
                self.line(format!(
                    "call i32 (i32, ptr, ...) @dprintf(i32 2, ptr @flurry_panic_format, i32 {}, ptr {})",
                    len32.text, data.text
                ));
                self.cleanup_boundary(source, iir::cleanup::Boundary::PredicateFailure)?;
                self.unwind_resources()?;
                if self.checked {
                    self.control_return(control::FAILURE);
                } else {
                    self.emit_trap();
                    self.line("unreachable");
                    self.terminated = true;
                }
                Ok(Value::unit())
            }
            _ => Err("unsupported host runtime control intrinsic".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_emitter<'a>(
        graph: &'a iir::Package,
        lambdas: &'a BTreeMap<usize, usize>,
        template_identities: &'a callables::TemplateIdentities<'a>,
        declarations: &'a mut BTreeSet<String>,
        text_constants: &'a mut strings::TextConstants,
        fuel: &'a mut usize,
    ) -> Emitter<'a> {
        Emitter {
            graph,
            cleanup: None,
            schedules: None,
            resource_unwinds: vec![],
            checked: true,
            function: None,
            closure_body: None,
            failure_result: Some(Ty::Unit),
            effects: vec![],
            effect_resume: None,
            capturing_resume: None,
            native_return: None,
            loop_destinations: Default::default(),
            outlined_return: None,
            outlined_loops: BTreeMap::new(),
            lambdas,
            template_identities,
            locals: BTreeMap::new(),
            live: BTreeMap::new(),
            temporaries: BTreeMap::new(),
            temporary_live: BTreeMap::new(),
            prepared: BTreeMap::new(),
            receipt_paths: vec![],
            pattern_orders: BTreeMap::new(),
            temporary_fields: BTreeMap::new(),
            loops: BTreeMap::new(),
            prologue: String::new(),
            body: String::new(),
            emitted_effects: attributes::EmittedBodyEffects::default(),
            target_triple: Some(Some("x86_64-pc-linux-gnu".into())),
            declarations,
            text_constants,
            attr_table: None,
            bitvec_cache: None,
            serial: 0,
            terminated: false,
            fuel,
        }
    }

    #[test]
    fn atomic_intrinsics_lower_and_execute_natively_and_trap_on_bad_ordering() {
        use nir::intrinsics::Operation;

        let graph = iir::Package {
            target: Some(nir::target::TargetData::new(64).unwrap()),
            ..Default::default()
        };
        let lambdas = BTreeMap::new();
        let template_identities = callables::TemplateIdentities::default();
        let mut declarations = BTreeSet::new();
        let mut text_constants = strings::TextConstants::default();
        let mut fuel = 100_000usize;
        let mut e = make_test_emitter(
            &graph,
            &lambdas,
            &template_identities,
            &mut declarations,
            &mut text_constants,
            &mut fuel,
        );

        let u64_ty = Ty::Int(64, false);
        let i32_ty = Ty::Int(32, true);
        let u8_ty = Ty::Int(8, false);
        let ord_seq = Value {
            ty: u8_ty.clone(),
            text: "4".into(),
        };
        let ord_acq = Value {
            ty: u8_ty.clone(),
            text: "1".into(),
        };
        let ord_rel = Value {
            ty: u8_ty.clone(),
            text: "2".into(),
        };
        let ord_rlx = Value {
            ty: u8_ty.clone(),
            text: "0".into(),
        };

        e.line("%cell = alloca i64, align 8");
        e.line("%exp = alloca i64, align 8");
        e.line("%scell = alloca i32, align 4");
        e.line("%bcell = alloca i8, align 1");
        e.line("%bexp = alloca i1, align 1");
        e.line("%pcell = alloca ptr, align 8");
        e.line("%pexp = alloca ptr, align 8");

        let cell = Value {
            ty: Ty::Pointer,
            text: "%cell".into(),
        };
        let exp = Value {
            ty: Ty::Pointer,
            text: "%exp".into(),
        };
        let scell = Value {
            ty: Ty::Pointer,
            text: "%scell".into(),
        };
        let bcell = Value {
            ty: Ty::Pointer,
            text: "%bcell".into(),
        };
        let bexp = Value {
            ty: Ty::Pointer,
            text: "%bexp".into(),
        };
        let pcell = Value {
            ty: Ty::Pointer,
            text: "%pcell".into(),
        };
        let pexp = Value {
            ty: Ty::Pointer,
            text: "%pexp".into(),
        };

        e.atomic_intrinsic(
            Operation::AtomicStore,
            &[
                cell.clone(),
                Value {
                    ty: u64_ty.clone(),
                    text: "10".into(),
                },
                ord_rel.clone(),
            ],
            Ty::Unit,
        )
        .unwrap();
        let prev_add = e
            .atomic_intrinsic(
                Operation::AtomicFetchAdd,
                &[
                    cell.clone(),
                    Value {
                        ty: u64_ty.clone(),
                        text: "5".into(),
                    },
                    ord_seq.clone(),
                ],
                u64_ty.clone(),
            )
            .unwrap();
        let prev_sub = e
            .atomic_intrinsic(
                Operation::AtomicFetchSub,
                &[
                    cell.clone(),
                    Value {
                        ty: u64_ty.clone(),
                        text: "3".into(),
                    },
                    ord_seq.clone(),
                ],
                u64_ty.clone(),
            )
            .unwrap();
        e.line("store i64 99, ptr %exp, align 8");
        let cas_fail = e
            .atomic_intrinsic(
                Operation::AtomicCompareExchange,
                &[
                    cell.clone(),
                    exp.clone(),
                    Value {
                        ty: u64_ty.clone(),
                        text: "77".into(),
                    },
                    ord_seq.clone(),
                    ord_acq.clone(),
                ],
                Ty::Bool,
            )
            .unwrap();
        let exp_after_fail = e.instruction(u64_ty.clone(), "load i64, ptr %exp, align 8".into());
        let cas_ok = e
            .atomic_intrinsic(
                Operation::AtomicCompareExchangeWeak,
                &[
                    cell.clone(),
                    exp.clone(),
                    Value {
                        ty: u64_ty.clone(),
                        text: "42".into(),
                    },
                    ord_seq.clone(),
                    ord_rlx.clone(),
                ],
                Ty::Bool,
            )
            .unwrap();
        let loaded = e
            .atomic_intrinsic(
                Operation::AtomicLoad,
                &[cell.clone(), ord_acq.clone()],
                u64_ty.clone(),
            )
            .unwrap();

        // Signed min/max vs unsigned min/max check.
        e.atomic_intrinsic(
            Operation::AtomicStore,
            &[
                scell.clone(),
                Value {
                    ty: i32_ty.clone(),
                    text: "-5".into(),
                },
                ord_seq.clone(),
            ],
            Ty::Unit,
        )
        .unwrap();
        let _ = e
            .atomic_intrinsic(
                Operation::AtomicFetchMax,
                &[
                    scell.clone(),
                    Value {
                        ty: i32_ty.clone(),
                        text: "2".into(),
                    },
                    ord_seq.clone(),
                ],
                i32_ty.clone(),
            )
            .unwrap();
        let signed_max_res = e
            .atomic_intrinsic(
                Operation::AtomicLoad,
                &[scell.clone(), ord_seq.clone()],
                i32_ty.clone(),
            )
            .unwrap();

        // Bool and Pointer atomics + dynamic ordering + fences.
        e.atomic_intrinsic(
            Operation::AtomicStore,
            &[
                bcell.clone(),
                Value {
                    ty: Ty::Bool,
                    text: "true".into(),
                },
                ord_seq.clone(),
            ],
            Ty::Unit,
        )
        .unwrap();
        e.line("store i1 true, ptr %bexp, align 1");
        let bcas = e
            .atomic_intrinsic(
                Operation::AtomicCompareExchange,
                &[
                    bcell.clone(),
                    bexp.clone(),
                    Value {
                        ty: Ty::Bool,
                        text: "false".into(),
                    },
                    ord_seq.clone(),
                    ord_acq.clone(),
                ],
                Ty::Bool,
            )
            .unwrap();
        let bload = e
            .atomic_intrinsic(Operation::AtomicLoad, &[bcell, ord_seq.clone()], Ty::Bool)
            .unwrap();

        e.atomic_intrinsic(
            Operation::AtomicStore,
            &[
                pcell.clone(),
                Value {
                    ty: Ty::Pointer,
                    text: "%cell".into(),
                },
                ord_seq.clone(),
            ],
            Ty::Unit,
        )
        .unwrap();
        e.line("store ptr %cell, ptr %pexp, align 8");
        let pcas = e
            .atomic_intrinsic(
                Operation::AtomicCompareExchange,
                &[
                    pcell.clone(),
                    pexp,
                    Value {
                        ty: Ty::Pointer,
                        text: "%scell".into(),
                    },
                    ord_seq.clone(),
                    ord_acq.clone(),
                ],
                Ty::Bool,
            )
            .unwrap();
        let pload = e
            .atomic_intrinsic(
                Operation::AtomicLoad,
                &[pcell, ord_seq.clone()],
                Ty::Pointer,
            )
            .unwrap();

        e.atomic_intrinsic(Operation::AtomicFence, &[ord_seq], Ty::Unit)
            .unwrap();
        e.atomic_intrinsic(Operation::AtomicCompilerFence, &[ord_acq], Ty::Unit)
            .unwrap();

        let ok1 = e.instruction(Ty::Bool, format!("icmp eq i64 {}, 10", prev_add.text));
        let ok2 = e.instruction(Ty::Bool, format!("icmp eq i64 {}, 15", prev_sub.text));
        let ok3 = e.instruction(Ty::Bool, format!("xor i1 {}, true", cas_fail.text));
        let ok4 = e.instruction(Ty::Bool, format!("icmp eq i64 {}, 12", exp_after_fail.text));
        let ok5 = cas_ok;
        let ok6 = e.instruction(Ty::Bool, format!("icmp eq i64 {}, 42", loaded.text));
        let ok7 = e.instruction(Ty::Bool, format!("icmp eq i32 {}, 2", signed_max_res.text));
        let ok8 = bcas;
        let ok9 = e.instruction(Ty::Bool, format!("xor i1 {}, true", bload.text));
        let ok10 = pcas;
        let ok11 = e.instruction(Ty::Bool, format!("icmp eq ptr {}, %scell", pload.text));
        let mut all_ok = ok1;
        for next in [ok2, ok3, ok4, ok5, ok6, ok7, ok8, ok9, ok10, ok11] {
            all_ok = e.instruction(Ty::Bool, format!("and i1 {}, {}", all_ok.text, next.text));
        }

        let module = format!(
            "declare void @llvm.trap()\ndefine i1 @run_atomics() {{\nentry:\n{}{}  ret i1 {}\n}}\ndefine i32 @main() {{\nentry:\n  %ok = call i1 @run_atomics()\n  %status = select i1 %ok, i32 0, i32 1\n  ret i32 %status\n}}\n",
            e.prologue, e.body, all_ok.text
        );
        assert!(module.contains("atomicrmw max ptr"), "{module}");
        assert!(
            module.contains("fence syncscope(\"singlethread\") acquire"),
            "{module}"
        );
        let tools = crate::host::Toolchain::default();
        for opt in [
            crate::host::OptimizationLevel::O0,
            crate::host::OptimizationLevel::O3,
        ] {
            let tc = crate::host::Toolchain {
                optimization: opt,
                ..tools.clone()
            };
            let output = tc.execute_test_ir(&module).unwrap();
            assert!(output.status.success(), "{opt:?}: {module}");
        }
    }
}
