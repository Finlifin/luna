//! Native optional data with tagged `{ i1, T }` fallback and niche optimization.
use super::*;

fn is_bool_carrier(ty: &Ty) -> bool {
    match ty {
        Ty::Bool => true,
        Ty::Enum(layout) => matches!(
            &layout.repr,
            enums::EnumRepr::SingleVariant { unwrapped: true, carrier, .. } if is_bool_carrier(carrier)
        ),
        _ => false,
    }
}

impl Emitter<'_> {
    pub(super) fn optional_absent_value(&mut self, opt_ty: &Ty) -> Result<Value, String> {
        let Ty::Optional(opt) = opt_ty else {
            return Err("host optional absent value requires an optional layout".into());
        };
        if let Some(niche) = &opt.niche {
            self.emit_niche_value(opt_ty.clone(), &opt.carrier, niche, niche.start)
        } else {
            Ok(Value {
                ty: opt_ty.clone(),
                text: "zeroinitializer".into(),
            })
        }
    }

    pub(super) fn optional_present_value(
        &mut self,
        opt_ty: &Ty,
        payload: Value,
    ) -> Result<Value, String> {
        let Ty::Optional(opt) = opt_ty else {
            return Err("host optional present value requires an optional layout".into());
        };
        if payload.ty != opt.payload {
            return Err("host optional payload representation mismatch".into());
        }
        if let Some(niche) = &opt.niche {
            let carried = if niche.widened_bool {
                self.emit_widen_bool_at_path(payload, &opt.carrier, &niche.llvm_path)?
            } else {
                payload
            };
            return Ok(Value {
                ty: opt_ty.clone(),
                text: carried.text,
            });
        }
        let tagged = self.instruction(
            opt_ty.clone(),
            format!("insertvalue {} zeroinitializer, i1 true, 0", opt_ty.llvm()),
        );
        if opt.payload == Ty::Unit {
            return Ok(tagged);
        }
        Ok(self.instruction(
            opt_ty.clone(),
            format!(
                "insertvalue {} {}, {} {}, 1",
                opt_ty.llvm(),
                tagged.text,
                payload.ty.llvm(),
                payload.text
            ),
        ))
    }

    pub(super) fn optional(&mut self, expr: &Expr, depth: usize) -> Result<Value, String> {
        let Term::Optional(payload_type) = &expr.ty else {
            return Err("host optional construction requires an optional type".into());
        };
        let ty = repr(self.graph, &expr.ty, self.checked)?;
        match &expr.kind {
            K::OptionalAbsent => self.optional_absent_value(&ty),
            K::OptionalPresent(payload) => {
                if payload.ty != **payload_type || payload.ty == Term::Primitive(P::Never) {
                    return Err("host optional payload type mismatch".into());
                }
                let payload_source = payload.source;
                let payload = self.expr(payload, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                let Ty::Optional(opt) = &ty else {
                    unreachable!()
                };
                if payload.ty != opt.payload {
                    return Err("host optional payload representation mismatch".into());
                }
                self.retain_prepared_field(expr.source, 0, payload_source, &payload)?;
                self.finish_prepared_construction(expr.source)?;
                self.optional_present_value(&ty, payload)
            }
            _ => Err("host optional construction node missing".into()),
        }
    }

    pub(super) fn optional_tag(&mut self, value: &Value) -> Result<Value, String> {
        let Ty::Optional(opt) = &value.ty else {
            return Err("host optional tag requires its layout".into());
        };
        if let Some(niche) = &opt.niche {
            self.emit_niche_is_payload(value, &opt.carrier, niche, 1)
        } else {
            Ok(self.instruction(
                Ty::Bool,
                format!("extractvalue {} {}, 0", value.ty.llvm(), value.text),
            ))
        }
    }

    // Only call after establishing presence in the current control-flow branch.
    pub(super) fn optional_payload(&mut self, value: &Value) -> Result<Value, String> {
        let Ty::Optional(opt) = &value.ty else {
            return Err("host optional payload layout missing".into());
        };
        if opt.payload == Ty::Unit {
            return Ok(Value::unit());
        }
        if let Some(niche) = &opt.niche {
            if niche.widened_bool {
                let raw = Value {
                    ty: opt.carrier.clone(),
                    text: value.text.clone(),
                };
                return self.emit_narrow_bool_at_path(raw, &opt.payload, &niche.llvm_path);
            }
            return Ok(Value {
                ty: opt.payload.clone(),
                text: value.text.clone(),
            });
        }
        Ok(self.instruction(
            opt.payload.clone(),
            format!("extractvalue {} {}, 1", value.ty.llvm(), value.text),
        ))
    }

    pub(super) fn emit_niche_value(
        &mut self,
        result_ty: Ty,
        carrier: &Ty,
        niche: &NicheEncoding,
        code: u64,
    ) -> Result<Value, String> {
        let leaf_ty = niche_leaf_ty(carrier, &niche.llvm_path)?;
        let leaf_text = match niche.scalar {
            nir::layout::NicheScalar::Pointer => "null".to_string(),
            nir::layout::NicheScalar::Int { .. } => code.to_string(),
        };
        if niche.llvm_path.is_empty() {
            return Ok(Value {
                ty: result_ty,
                text: leaf_text,
            });
        }
        let indices = niche
            .llvm_path
            .iter()
            .map(|i| format!(", {i}"))
            .collect::<String>();
        Ok(self.instruction(
            result_ty,
            format!(
                "insertvalue {} zeroinitializer, {} {leaf_text}{indices}",
                carrier.llvm(),
                leaf_ty.llvm()
            ),
        ))
    }

    pub(super) fn extract_niche_leaf(
        &mut self,
        value: &Value,
        carrier: &Ty,
        niche: &NicheEncoding,
    ) -> Result<Value, String> {
        let leaf_ty = niche_leaf_ty(carrier, &niche.llvm_path)?;
        if niche.llvm_path.is_empty() {
            return Ok(Value {
                ty: leaf_ty,
                text: value.text.clone(),
            });
        }
        let indices = niche
            .llvm_path
            .iter()
            .map(|i| format!(", {i}"))
            .collect::<String>();
        Ok(self.instruction(
            leaf_ty,
            format!("extractvalue {} {}{indices}", carrier.llvm(), value.text),
        ))
    }

    pub(super) fn emit_niche_equals_code(
        &mut self,
        value: &Value,
        carrier: &Ty,
        niche: &NicheEncoding,
        code: u64,
    ) -> Result<Value, String> {
        let leaf = self.extract_niche_leaf(value, carrier, niche)?;
        let rhs = match niche.scalar {
            nir::layout::NicheScalar::Pointer => "null".to_string(),
            nir::layout::NicheScalar::Int { .. } => code.to_string(),
        };
        Ok(self.instruction(
            Ty::Bool,
            format!("icmp eq {} {}, {rhs}", leaf.ty.llvm(), leaf.text),
        ))
    }

    pub(super) fn emit_niche_is_payload(
        &mut self,
        value: &Value,
        carrier: &Ty,
        niche: &NicheEncoding,
        empty_count: u64,
    ) -> Result<Value, String> {
        let leaf = self.extract_niche_leaf(value, carrier, niche)?;
        let leaf_llvm = leaf.ty.llvm();
        if empty_count == 1 {
            let rhs = match niche.scalar {
                nir::layout::NicheScalar::Pointer => "null".to_string(),
                nir::layout::NicheScalar::Int { .. } => niche.start.to_string(),
            };
            return Ok(self.instruction(
                Ty::Bool,
                format!("icmp ne {leaf_llvm} {}, {rhs}", leaf.text),
            ));
        }
        let nir::layout::NicheScalar::Int { bits } = niche.scalar else {
            return Err("host pointer niche cannot encode multiple empty variants".into());
        };
        let max_val = if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        let end_incl = niche.start + (empty_count - 1);
        if niche.start == 0 {
            Ok(self.instruction(
                Ty::Bool,
                format!("icmp ugt {leaf_llvm} {}, {end_incl}", leaf.text),
            ))
        } else if end_incl == max_val {
            Ok(self.instruction(
                Ty::Bool,
                format!("icmp ult {leaf_llvm} {}, {}", leaf.text, niche.start),
            ))
        } else {
            let below = self.instruction(
                Ty::Bool,
                format!("icmp ult {leaf_llvm} {}, {}", leaf.text, niche.start),
            );
            let above = self.instruction(
                Ty::Bool,
                format!("icmp ugt {leaf_llvm} {}, {end_incl}", leaf.text),
            );
            Ok(self.instruction(Ty::Bool, format!("or i1 {}, {}", below.text, above.text)))
        }
    }

    fn rebuild_record_with_field(
        &mut self,
        value: &Value,
        src_fields: &[Ty],
        dst_fields: &[Ty],
        target_ty: Ty,
        replace_idx: usize,
        replace_rest: &[usize],
        widen: bool,
    ) -> Result<Value, String> {
        if src_fields.len() != dst_fields.len() || replace_idx >= src_fields.len() {
            return Err("host niche record rebuild field count mismatch".into());
        }
        let src_llvm = value.ty.llvm();
        let dst_llvm = target_ty.llvm();
        let mut result = Value {
            ty: target_ty,
            text: "zeroinitializer".into(),
        };
        for (j, (src_f, dst_f)) in src_fields.iter().zip(dst_fields.iter()).enumerate() {
            if *src_f == Ty::Unit {
                continue;
            }
            let extracted = self.instruction(
                src_f.clone(),
                format!("extractvalue {src_llvm} {}, {j}", value.text),
            );
            let transformed = if j == replace_idx {
                if widen {
                    self.emit_widen_bool_at_path(extracted, dst_f, replace_rest)?
                } else {
                    self.emit_narrow_bool_at_path(extracted, dst_f, replace_rest)?
                }
            } else {
                extracted
            };
            result = self.instruction(
                result.ty,
                format!(
                    "insertvalue {dst_llvm} {}, {} {}, {j}",
                    result.text,
                    transformed.ty.llvm(),
                    transformed.text
                ),
            );
        }
        Ok(result)
    }

    pub(super) fn emit_widen_bool_at_path(
        &mut self,
        value: Value,
        target_ty: &Ty,
        path: &[usize],
    ) -> Result<Value, String> {
        if path.is_empty() {
            if !is_bool_carrier(&value.ty) {
                return Err("host niche bool-widen leaf must be bool".into());
            }
            return Ok(self.instruction(target_ty.clone(), format!("zext i1 {} to i8", value.text)));
        }
        let idx = path[0];
        let rest = &path[1..];
        match (value.ty.clone(), target_ty) {
            (Ty::Record(src_fields), Ty::Record(dst_fields)) => self.rebuild_record_with_field(
                &value,
                &src_fields,
                dst_fields,
                target_ty.clone(),
                idx,
                rest,
                true,
            ),
            (Ty::Optional(inner), Ty::Record(dst_fields))
                if inner.niche.is_none() && idx == 0 && rest.is_empty() =>
            {
                let src_llvm = value.ty.llvm();
                let dst_llvm = target_ty.llvm();
                let tag1 = self.instruction(
                    Ty::Bool,
                    format!("extractvalue {src_llvm} {}, 0", value.text),
                );
                let tag8 =
                    self.instruction(Ty::Int(8, false), format!("zext i1 {} to i8", tag1.text));
                let mut res = self.instruction(
                    target_ty.clone(),
                    format!(
                        "insertvalue {dst_llvm} zeroinitializer, i8 {}, 0",
                        tag8.text
                    ),
                );
                if inner.payload != Ty::Unit {
                    let pay = self.instruction(
                        dst_fields[1].clone(),
                        format!("extractvalue {src_llvm} {}, 1", value.text),
                    );
                    res = self.instruction(
                        target_ty.clone(),
                        format!(
                            "insertvalue {dst_llvm} {}, {} {}, 1",
                            res.text,
                            pay.ty.llvm(),
                            pay.text
                        ),
                    );
                }
                Ok(res)
            }
            (Ty::Enum(layout), _) => match &layout.repr {
                enums::EnumRepr::SingleVariant {
                    unwrapped, carrier, ..
                } => {
                    if *unwrapped {
                        let inner_val = Value {
                            ty: carrier.clone(),
                            text: value.text,
                        };
                        self.emit_widen_bool_at_path(inner_val, target_ty, path)
                    } else if let (Ty::Record(src_fields), Ty::Record(dst_fields)) =
                        (carrier, target_ty)
                    {
                        self.rebuild_record_with_field(
                            &value,
                            src_fields,
                            dst_fields,
                            target_ty.clone(),
                            idx,
                            rest,
                            true,
                        )
                    } else {
                        Err("host niche bool-widen enum carrier mismatch".into())
                    }
                }
                _ => Err("host niche bool-widen unsupported enum representation".into()),
            },
            _ => Err("host niche bool-widen path mismatch".into()),
        }
    }

    pub(super) fn emit_narrow_bool_at_path(
        &mut self,
        value: Value,
        target_ty: &Ty,
        path: &[usize],
    ) -> Result<Value, String> {
        if path.is_empty() {
            if !is_bool_carrier(target_ty) {
                return Err("host niche bool-narrow target must be bool".into());
            }
            return Ok(
                self.instruction(target_ty.clone(), format!("trunc i8 {} to i1", value.text))
            );
        }
        let idx = path[0];
        let rest = &path[1..];
        match (value.ty.clone(), target_ty) {
            (Ty::Record(src_fields), Ty::Record(dst_fields)) => self.rebuild_record_with_field(
                &value,
                &src_fields,
                dst_fields,
                target_ty.clone(),
                idx,
                rest,
                false,
            ),
            (Ty::Record(_), Ty::Optional(inner))
                if inner.niche.is_none() && idx == 0 && rest.is_empty() =>
            {
                let src_llvm = value.ty.llvm();
                let dst_llvm = target_ty.llvm();
                let tag8 = self.instruction(
                    Ty::Int(8, false),
                    format!("extractvalue {src_llvm} {}, 0", value.text),
                );
                let tag1 = self.instruction(Ty::Bool, format!("trunc i8 {} to i1", tag8.text));
                let mut res = self.instruction(
                    target_ty.clone(),
                    format!(
                        "insertvalue {dst_llvm} zeroinitializer, i1 {}, 0",
                        tag1.text
                    ),
                );
                if inner.payload != Ty::Unit {
                    let pay = self.instruction(
                        inner.payload.clone(),
                        format!("extractvalue {src_llvm} {}, 1", value.text),
                    );
                    res = self.instruction(
                        target_ty.clone(),
                        format!(
                            "insertvalue {dst_llvm} {}, {} {}, 1",
                            res.text,
                            pay.ty.llvm(),
                            pay.text
                        ),
                    );
                }
                Ok(res)
            }
            (_, Ty::Enum(layout)) => match &layout.repr {
                enums::EnumRepr::SingleVariant {
                    unwrapped, carrier, ..
                } => {
                    if *unwrapped {
                        let mut inner_val = self.emit_narrow_bool_at_path(value, carrier, path)?;
                        inner_val.ty = target_ty.clone();
                        Ok(inner_val)
                    } else if let (Ty::Record(src_fields), Ty::Record(dst_fields)) =
                        (&value.ty, carrier)
                    {
                        self.rebuild_record_with_field(
                            &value,
                            src_fields,
                            dst_fields,
                            target_ty.clone(),
                            idx,
                            rest,
                            false,
                        )
                    } else {
                        Err("host niche bool-narrow enum carrier mismatch".into())
                    }
                }
                _ => Err("host niche bool-narrow unsupported enum representation".into()),
            },
            _ => Err("host niche bool-narrow path mismatch".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::{MatchArm, Pattern, SourceRange};

    fn expr(ty: Term, kind: K) -> Expr {
        Expr {
            ty,
            kind,
            source: SourceRange { start: 1, end: 2 },
        }
    }

    #[test]
    fn optional_layout_keeps_layers_and_bounds_recursion() {
        let graph = iir::Package::default();
        let never = Term::Primitive(P::Never);
        assert!(repr(&graph, &never, true).is_err());
        assert_eq!(
            repr(&graph, &Term::Optional(Box::new(never)), true)
                .unwrap()
                .llvm(),
            "{ i1, {} }"
        );
        let opt_bool = Term::Optional(Box::new(Term::Primitive(P::Bool)));
        assert_eq!(repr(&graph, &opt_bool, true).unwrap().llvm(), "i8");
        let opt2_bool = Term::Optional(Box::new(opt_bool));
        assert_eq!(repr(&graph, &opt2_bool, true).unwrap().llvm(), "i8");

        let opt_ptr = Term::Optional(Box::new(Term::Pointer(Box::new(Term::Primitive(P::Bool)))));
        assert_eq!(repr(&graph, &opt_ptr, true).unwrap().llvm(), "ptr");
        let opt2_ptr = Term::Optional(Box::new(opt_ptr));
        assert_eq!(repr(&graph, &opt2_ptr, true).unwrap().llvm(), "{ i1, ptr }");

        assert_eq!(
            repr(
                &graph,
                &Term::Optional(Box::new(Term::Primitive(P::Char))),
                true
            )
            .unwrap()
            .llvm(),
            "i32"
        );
        let opt_u64 = Term::Optional(Box::new(Term::Primitive(P::U64)));
        assert_eq!(repr(&graph, &opt_u64, true).unwrap().llvm(), "{ i1, i64 }");
        let opt2_u64 = Term::Optional(Box::new(opt_u64));
        assert_eq!(repr(&graph, &opt2_u64, true).unwrap().llvm(), "{ i8, i64 }");

        let mut ty = Term::Optional(Box::new(Term::Primitive(P::U64)));
        for _ in 0..128 {
            ty = Term::Optional(Box::new(ty));
        }
        assert!(repr(&graph, &ty, true).is_err());
        assert!(
            repr(
                &graph,
                &Term::Optional(Box::new(Term::Pointer(Box::new(Term::Primitive(P::Bool))))),
                false
            )
            .is_err()
        );
    }

    #[test]
    fn native_optional_construction_and_patterns_preserve_all_states() {
        let graph = iir::Package::default();
        let lambdas = BTreeMap::new();
        let template_identities = callables::TemplateIdentities::default();
        let boolean = Term::Primitive(P::Bool);
        let optional = Term::Optional(Box::new(boolean.clone()));
        let nested = Term::Optional(Box::new(optional.clone()));
        let patterns = [
            Pattern::OptionalAbsent,
            Pattern::OptionalPresent(Box::new(Pattern::OptionalAbsent)),
            Pattern::OptionalPresent(Box::new(Pattern::OptionalPresent(Box::new(
                Pattern::Literal(Literal::Bool(false)),
            )))),
            Pattern::OptionalPresent(Box::new(Pattern::OptionalPresent(Box::new(
                Pattern::Literal(Literal::Bool(true)),
            )))),
        ];
        for state in 0..4 {
            let mut declarations = BTreeSet::new();
            let mut fuel = 10_000;
            let mut emitter = Emitter {
                graph: &graph,
                cleanup: None,
                schedules: None,
                resource_unwinds: vec![],
                checked: true,
                function: None,
                closure_body: None,
                failure_result: None,
                effects: vec![],
                effect_resume: None,
                native_return: None,
                loop_destinations: Default::default(),
                outlined_return: None,
                outlined_loops: BTreeMap::new(),
                lambdas: &lambdas,
                template_identities: &template_identities,
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
                declarations: &mut declarations,
                text_constants: &mut strings::TextConstants::default(),
                attr_table: None,
                bitvec_cache: None,
                serial: 0,
                terminated: false,
                fuel: &mut fuel,
            };

            let input = if state == 0 {
                expr(nested.clone(), K::OptionalAbsent)
            } else {
                let inner = if state == 1 {
                    expr(optional.clone(), K::OptionalAbsent)
                } else {
                    expr(
                        optional.clone(),
                        K::OptionalPresent(Box::new(expr(
                            boolean.clone(),
                            K::Literal(Literal::Bool(state == 3)),
                        ))),
                    )
                };
                expr(nested.clone(), K::OptionalPresent(Box::new(inner)))
            };
            let arms = patterns
                .iter()
                .enumerate()
                .map(|(index, pattern)| MatchArm {
                    pattern: pattern.clone(),
                    guard: None,
                    body: expr(boolean.clone(), K::Literal(Literal::Bool(index == state))),
                })
                .collect();
            let matched = expr(
                boolean.clone(),
                K::Match {
                    value: Box::new(input),
                    arms,
                },
            );
            let value = emitter.expr(&matched, 0).unwrap();
            emitter.guard(&value).unwrap();
            emitter.line("ret i32 0");
            let ir = format!(
                "declare void @llvm.trap()\ndefine i32 @main() {{\nentry:\n{}{}}}\n",
                emitter.prologue, emitter.body
            );
            for optimization in [
                crate::host::OptimizationLevel::O0,
                crate::host::OptimizationLevel::O3,
            ] {
                let toolchain = crate::host::Toolchain {
                    optimization,
                    ..Default::default()
                };
                let output = toolchain.execute_test_ir(&ir).unwrap();
                assert!(
                    output.status.success(),
                    "optional state {state}: {output:?}"
                );
            }
        }
    }
}
