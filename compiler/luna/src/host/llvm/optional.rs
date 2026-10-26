//! Native optional data. The tag is local host storage, not a library enum ABI.
use super::*;

impl Emitter<'_> {
    pub(super) fn optional(&mut self, expr: &Expr, depth: usize) -> Result<Value, String> {
        let Term::Optional(payload_type) = &expr.ty else {
            return Err("host optional construction requires an optional type".into());
        };
        let ty = repr(self.graph, &expr.ty, self.checked)?;
        match &expr.kind {
            K::OptionalAbsent => Ok(Value {
                ty,
                text: "zeroinitializer".into(),
            }),
            K::OptionalPresent(payload) => {
                if payload.ty != **payload_type || payload.ty == Term::Primitive(P::Never) {
                    return Err("host optional payload type mismatch".into());
                }
                let payload_source = payload.source;
                let payload = self.expr(payload, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                let Ty::Optional(expected) = &ty else {
                    unreachable!()
                };
                if payload.ty != **expected {
                    return Err("host optional payload representation mismatch".into());
                }
                self.retain_prepared_field(expr.source, 0, payload_source, &payload)?;
                self.finish_prepared_construction(expr.source)?;
                let tagged = self.instruction(
                    ty.clone(),
                    format!("insertvalue {} zeroinitializer, i1 true, 0", ty.llvm()),
                );
                if payload.ty == Ty::Unit {
                    return Ok(tagged);
                }
                Ok(self.instruction(
                    ty.clone(),
                    format!(
                        "insertvalue {} {}, {} {}, 1",
                        ty.llvm(),
                        tagged.text,
                        payload.ty.llvm(),
                        payload.text
                    ),
                ))
            }
            _ => Err("host optional construction node missing".into()),
        }
    }

    pub(super) fn optional_tag(&mut self, value: &Value) -> Result<Value, String> {
        if !matches!(value.ty, Ty::Optional(_)) {
            return Err("host optional tag requires its layout".into());
        }
        Ok(self.instruction(
            Ty::Bool,
            format!("extractvalue {} {}, 0", value.ty.llvm(), value.text),
        ))
    }

    // Only call after establishing presence in the current control-flow branch.
    pub(super) fn optional_payload(&mut self, value: &Value) -> Result<Value, String> {
        let Ty::Optional(payload) = &value.ty else {
            return Err("host optional payload layout missing".into());
        };
        if **payload == Ty::Unit {
            return Ok(Value::unit());
        }
        Ok(self.instruction(
            (**payload).clone(),
            format!("extractvalue {} {}, 1", value.ty.llvm(), value.text),
        ))
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
        let mut ty = Term::Optional(Box::new(Term::Primitive(P::Bool)));
        ty = Term::Optional(Box::new(ty));
        assert_eq!(
            repr(&graph, &ty, true).unwrap().llvm(),
            "{ i1, { i1, i1 } }"
        );
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
                declarations: &mut declarations,
                text_constants: &mut strings::TextConstants::default(),
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
