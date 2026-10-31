//! Lexically isolated cleanup frames for default argument evaluation.
use super::frames::UnwindFrame;
use super::*;
use iir::cleanup::{Boundary, ContextKey};

impl Emitter<'_> {
    pub(super) fn eval_default(
        &mut self,
        construction: nir::program::SourceRange,
        argument: &nir::program::CallArgument,
        bindings: BTreeMap<LocalId, Slot>,
        caller: BTreeMap<LocalId, Slot>,
        depth: usize,
    ) -> Result<Value, String> {
        if self.cleanup.is_none() {
            return self.expr(&argument.value, depth);
        }
        let mut state = self.enter_local_scope(ContextKey::Default {
            construction,
            parameter: argument.parameter,
        })?;
        state.locals = caller;
        self.resource_unwinds.push(UnwindFrame {
            state,
            source: construction,
            boundary: Boundary::DefaultFailure(argument.parameter),
        });
        self.locals = bindings;
        let result = self.expr(&argument.value, depth);
        let frame = self
            .resource_unwinds
            .pop()
            .ok_or("host default cleanup frame missing")?;
        self.restore_local_frame(frame.state);
        if self.terminated {
            return Err("host default nonlocal control transfer is not implemented".into());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iir::cleanup::{Action, FunctionSchedule, Program, Site, Step, Target};
    use nir::program::{FunctionPhase, SourceRange};

    #[test]
    fn native_default_failure_unwinds_inner_locals_before_outer_prepared_values() {
        let source = |start| SourceRange {
            start,
            end: start + 1,
        };
        let binding = LocalId { owner: 0, index: 0 };
        let declaration = nir::DeclarationId {
            package: 0,
            local: 0,
        };
        let key = iir::InstanceKey {
            declaration,
            arguments: vec![].into(),
        };
        let byte = Term::Primitive(P::U8);
        let destructor = Function {
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration,
            name: "observe_cleanup".into(),
            signature: Term::Function {
                parameters: vec![nir::Parameter {
                    sort: Sort::Runtime,
                    ty: Term::Pointer(Box::new(byte.clone())),
                }],
                result: Box::new(Term::Primitive(P::Void)),
            },
            parameters: vec![binding],
            body: Expr {
                source: source(0),
                ty: Term::Primitive(P::Void),
                kind: K::Unit,
            },
            defaults: vec![],
        };
        let graph = iir::Package {
            functions: vec![iir::FunctionInstance {
                key: key.clone(),
                function: destructor,
            }],
            ..Default::default()
        };
        let program = Program {
            ty: byte.clone(),
            guards: vec![],
            steps: vec![Step::Invoke {
                path: vec![],
                subject: byte,
                method: key.clone(),
            }],
        };
        let schedule = |at, boundary, target| FunctionSchedule {
            instance: key.clone(),
            context: vec![],
            transfers: vec![],
            copies: vec![],
            pattern_transfers: vec![],
            pattern_orders: vec![],
            discards: vec![],
            sites: vec![Site {
                source: source(at),
                boundary,
                actions: vec![Action {
                    target,
                    program: program.clone(),
                }],
            }],
        };
        let outer = schedule(
            10,
            Boundary::DefaultFailure(0),
            Target::Prepared {
                construction: source(10),
                slot: 0,
                value_source: source(10),
            },
        );
        let middle = schedule(
            20,
            Boundary::DefaultFailure(1),
            Target::Local {
                binding,
                fields: vec![],
            },
        );
        let current = schedule(
            30,
            Boundary::PredicateFailure,
            Target::Local {
                binding,
                fields: vec![],
            },
        );
        let lambdas = BTreeMap::new();
        let scope_key = |index| ContextKey::EffectHandler {
            installation: source(90),
            handler: 0,
            arm: index,
        };
        let mut child = current.clone();
        child.context = vec![scope_key(0)];
        let mut duplicate = child.clone();
        duplicate.context = vec![scope_key(1)];
        let published = iir::cleanup::Schedule {
            functions: vec![child, duplicate.clone(), duplicate],
            ..Default::default()
        };
        let template_identities = callables::TemplateIdentities::default();
        for passed in [false, true] {
            let mut declarations = BTreeSet::new();
            let mut fuel = 10_000;
            let mut emitter = Emitter {
                graph: &graph,
                cleanup: Some(&outer),
                schedules: Some(&published),
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
            let value = |text: &str| Value {
                ty: Ty::Int(8, false),
                text: text.into(),
            };
            emitter
                .retain_prepared_field(source(10), 0, source(10), &value("65"))
                .unwrap();
            let outer_state = emitter.take_local_frame();
            emitter.resource_unwinds.push(UnwindFrame {
                state: outer_state,
                source: source(10),
                boundary: Boundary::DefaultFailure(0),
            });
            emitter.cleanup = Some(&middle);
            emitter.bind(binding, &value("66")).unwrap();
            let middle_state = emitter.take_local_frame();
            emitter.resource_unwinds.push(UnwindFrame {
                state: middle_state,
                source: source(20),
                boundary: Boundary::DefaultFailure(1),
            });
            emitter.cleanup = Some(&current);
            emitter.bind(binding, &value("67")).unwrap();
            let current_pointer = emitter.locals[&binding].pointer.clone();
            for index in [1, 2] {
                assert!(emitter.enter_local_scope(scope_key(index)).is_err());
                assert!(std::ptr::eq(emitter.cleanup.unwrap(), &current));
                assert_eq!(emitter.locals[&binding].pointer, current_pointer);
            }
            let saved_fuel = *emitter.fuel;
            *emitter.fuel = 0;
            assert!(emitter.enter_local_scope(scope_key(0)).is_err());
            assert!(std::ptr::eq(emitter.cleanup.unwrap(), &current));
            assert_eq!(emitter.locals[&binding].pointer, current_pointer);
            *emitter.fuel = saved_fuel;
            let suspended = emitter.enter_local_scope(scope_key(0)).unwrap();
            assert!(emitter.locals.is_empty());
            emitter.bind(binding, &value("68")).unwrap();
            assert_ne!(emitter.locals[&binding].pointer, current_pointer);
            emitter.restore_local_frame(suspended);
            assert!(std::ptr::eq(emitter.cleanup.unwrap(), &current));
            assert_eq!(emitter.locals[&binding].pointer, current_pointer);
            emitter
                .guard_with_cleanup(
                    &Value {
                        ty: Ty::Bool,
                        text: passed.to_string(),
                    },
                    Some((source(30), Boundary::PredicateFailure)),
                )
                .unwrap();
            assert!(std::ptr::eq(emitter.cleanup.unwrap(), &current));
            assert_eq!(emitter.locals[&binding].pointer, current_pointer);
            assert_eq!(emitter.resource_unwinds.len(), 2);
            emitter.line("ret i32 0");
            // A low-level observer writes each owner byte without buffering.
            // This tests cleanup generation, not source P3 admission.
            let ir = format!(
                "declare void @llvm.trap()\ndeclare i64 @write(i32, ptr, i64)\ndefine void @flurry_0(ptr %p) {{\nentry:\n  %n = call i64 @write(i32 1, ptr %p, i64 1)\n  ret void\n}}\ndefine i32 @main() {{\nentry:\n{}{}}}\n",
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
                let out = toolchain.execute_test_ir(&ir).unwrap();
                assert_eq!(out.status.success(), passed);
                assert_eq!(
                    out.stdout,
                    if passed {
                        b"".as_slice()
                    } else {
                        b"CBA".as_slice()
                    }
                );
            }
        }
    }
}
