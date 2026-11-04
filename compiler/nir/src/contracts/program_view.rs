//! Borrowed executable parts of a declaration or an interface contract.
//! Inspection does not specialize the contract or invent an executable body.
use crate::{
    DeclarationId, Term, TypeError,
    program::{EntrySnapshot, Expr, Function, FunctionPhase, LocalId, MethodContract, SourceRange},
};
use std::fmt;

#[derive(Clone, Copy)]
pub struct CallableProgram<'a> {
    pub(crate) declaration: DeclarationId,
    pub(crate) phase: FunctionPhase,
    pub(crate) is_unsafe: bool,
    pub(crate) signature: &'a Term,
    pub(crate) type_parameters: &'a [DeclarationId],
    pub(crate) parameters: &'a [LocalId],
    pub(crate) body: Option<&'a Expr>,
    pub(crate) source: SourceRange,
    pub(crate) defaults: &'a [(usize, Expr)],
    pub(crate) preconditions: &'a [Expr],
    pub(crate) postconditions: &'a [Expr],
    pub(crate) result_binding: Option<LocalId>,
    pub(crate) entry_snapshots: &'a [EntrySnapshot],
}

impl<'a> CallableProgram<'a> {
    pub fn function(function: &'a Function) -> Self {
        Self {
            declaration: function.declaration,
            phase: function.phase,
            is_unsafe: function.is_unsafe,
            signature: &function.signature,
            type_parameters: &function.type_parameters,
            parameters: &function.parameters,
            body: Some(&function.body),
            source: function.body.source,
            defaults: &function.defaults,
            preconditions: &function.preconditions,
            postconditions: &function.postconditions,
            result_binding: function.result_binding,
            entry_snapshots: &function.entry_snapshots,
        }
    }

    /// Contextual code is not an ordinary callable. Its code interfaces are
    /// checked separately; this adapter cannot erase their category or ports.
    pub fn method(method: &'a MethodContract) -> Result<Self, TypeError> {
        let signature = method.signature.ordinary()?;
        Ok(Self {
            declaration: method.declaration,
            phase: if signature.is_comptime_callable() {
                FunctionPhase::Comptime
            } else {
                FunctionPhase::Runtime
            },
            is_unsafe: method.is_unsafe,
            signature: signature.callable_signature(),
            type_parameters: &method.type_parameters,
            parameters: &method.parameter_bindings,
            body: None,
            source: SourceRange { start: 0, end: 0 },
            defaults: &[],
            preconditions: &method.program.preconditions,
            postconditions: &method.program.postconditions,
            result_binding: method.program.result_binding,
            entry_snapshots: &method.program.entry_snapshots,
        })
    }

    /// Same root order as a Function traversal; consumers can reverse it to
    /// retain the existing work-stack order without collecting every root.
    pub fn roots(self) -> impl DoubleEndedIterator<Item = &'a Expr> {
        self.body
            .into_iter()
            .chain(self.preconditions)
            .chain(self.postconditions)
            .chain(self.defaults.iter().map(|(_, value)| value))
    }
}

impl fmt::Display for CallableProgram<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(callable-program {} (phase {}) (unsafe {}) (signature {})",
            self.declaration, self.phase, self.is_unsafe, self.signature
        )?;
        f.write_str(" (type-parameters")?;
        for parameter in self.type_parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (parameters")?;
        for parameter in self.parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (requires")?;
        for condition in self.preconditions {
            write!(f, " {condition}")?;
        }
        f.write_str(") (ensures")?;
        for condition in self.postconditions {
            write!(f, " {condition}")?;
        }
        f.write_str(") (result-binding")?;
        if let Some(binding) = self.result_binding {
            write!(f, " {binding}")?;
        }
        f.write_str(") (entry-snapshots")?;
        for snapshot in self.entry_snapshots {
            write!(f, " {snapshot}")?;
        }
        f.write_str(") (defaults")?;
        for (index, value) in self.defaults {
            write!(f, " ({index} {value})")?;
        }
        f.write_str(") (body")?;
        if let Some(body) = self.body {
            write!(f, " {body}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::{ExprKind as K, Literal};
    use crate::{Primitive, Sort};

    fn flag(index: u32) -> Expr {
        Expr {
            source: SourceRange {
                start: index,
                end: index + 1,
            },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(Literal::Bool(true)),
        }
    }

    fn method() -> MethodContract {
        MethodContract {
            declaration: DeclarationId {
                package: 1,
                local: 2,
            },
            name: "check\"value".into(),
            phase: Default::default(),
            is_quote: false,
            pure: true,
            is_unsafe: false,
            type_parameters: vec![DeclarationId {
                package: 1,
                local: 3,
            }],
            type_arguments: vec![],
            signature: Term::Function {
                parameters: vec![crate::Parameter {
                    sort: Sort::Runtime,
                    ty: Term::Primitive(Primitive::Bool),
                }],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            }
            .into(),
            parameter_bindings: vec![LocalId { owner: 2, index: 0 }],
            capabilities: vec![],
            requirements: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            default: None,
            program: super::super::ProgramContract {
                preconditions: vec![flag(10)],
                postconditions: vec![flag(20)],
                result_binding: Some(LocalId { owner: 2, index: 1 }),
                entry_snapshots: vec![EntrySnapshot {
                    parameter: LocalId { owner: 2, index: 0 },
                    binding: LocalId { owner: 2, index: 2 },
                }],
            },
        }
    }

    #[test]
    fn method_inspection_borrows_original_contract_without_a_synthetic_body() {
        let method = method();
        let view = CallableProgram::method(&method).unwrap();
        assert!(std::ptr::eq(
            view.signature,
            method.signature.ordinary().unwrap()
        ));
        assert!(std::ptr::eq(
            view.parameters,
            method.parameter_bindings.as_slice()
        ));
        assert!(std::ptr::eq(
            view.type_parameters,
            method.type_parameters.as_slice()
        ));
        assert!(std::ptr::eq(
            view.entry_snapshots,
            method.program.entry_snapshots.as_slice()
        ));
        let roots: Vec<_> = view.roots().collect();
        assert_eq!(roots.len(), 2);
        assert!(std::ptr::eq(roots[0], &method.program.preconditions[0]));
        assert!(std::ptr::eq(roots[1], &method.program.postconditions[0]));
        assert!(view.body.is_none());
        assert!(view.defaults.is_empty());
        assert_eq!(view.result_binding, method.program.result_binding);
        let mut visited = vec![];
        crate::visit::program_expressions(view, &mut 2, |expr, _| {
            visited.push(expr);
            Ok(())
        })
        .unwrap();
        assert!(std::ptr::eq(visited[0], roots[1]));
        assert!(std::ptr::eq(visited[1], roots[0]));
        let dump = view.to_string();
        assert!(dump.starts_with("(callable-program "));
        assert!(dump.contains("(requires "));
        assert!(dump.contains("(ensures "));
        assert!(dump.ends_with("(body))"));
    }

    #[test]
    fn ordinary_program_roots_include_defaults_body_and_both_contract_sides() {
        let mut function = method()
            .program_summary_declaration_inputs(&[], &[], &[], &[], &mut 1000)
            .unwrap();
        function.body = flag(30);
        function.defaults = vec![(0, flag(40))];
        let view = CallableProgram::function(&function);
        assert!(std::ptr::eq(view.body.unwrap(), &function.body));
        let mut visited = vec![];
        crate::visit::program_expressions(view, &mut 4, |expr, _| {
            visited.push(expr.source.start);
            Ok(())
        })
        .unwrap();
        assert_eq!(visited, [40, 20, 10, 30]);
        let dump = view.to_string();
        assert!(dump.contains("(defaults (0 "));
        assert!(!dump.ends_with("(body))"));
    }

    #[test]
    fn contract_roots_share_fuel_and_retry_from_original_expressions() {
        let mut method = method();
        method.program.preconditions = (0..500).map(flag).collect();
        method.program.postconditions = (500..1000).map(flag).collect();
        let view = CallableProgram::method(&method).unwrap();
        let mut seen = 0;
        let mut fuel = 9;
        let error = crate::visit::program_expressions(view, &mut fuel, |_, _| {
            seen += 1;
            Ok(())
        })
        .unwrap_err();
        assert!(error.contains("budget exhausted"));
        assert_eq!((seen, fuel), (9, 0));
        seen = 0;
        crate::visit::program_expressions(view, &mut 1000, |_, _| {
            seen += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, 1000);
    }

    #[test]
    fn contract_call_safety_preserves_phase_and_lexical_unsafe_boundaries() {
        let mut method = method();
        method.signature =
            Term::ComptimeFunction(Box::new(method.signature.ordinary().unwrap().clone())).into();
        let call = Expr {
            kind: K::Call {
                callee: Box::new(Expr {
                    ty: Term::UnsafeFunction(Box::new(Term::Function {
                        parameters: vec![],
                        result: Box::new(Term::Primitive(Primitive::Bool)),
                    })),
                    kind: K::Function(DeclarationId {
                        package: 1,
                        local: 4,
                    }),
                    ..flag(70)
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
            ..flag(71)
        };
        method.program.postconditions = vec![Expr {
            kind: K::UnsafeScope(Box::new(call.clone())),
            ..flag(72)
        }];
        let view = CallableProgram::method(&method).unwrap();
        assert_eq!(view.phase, FunctionPhase::Comptime);
        crate::call_safety::validate_program(view, &mut 100).unwrap();
        // A scope in the postcondition cannot authorize a sibling precondition.
        method.program.preconditions = vec![call];
        let view = CallableProgram::method(&method).unwrap();
        assert!(
            crate::call_safety::validate_program(view, &mut 100)
                .unwrap_err()
                .contains("71..72")
        );
        method.is_unsafe = true;
        crate::call_safety::validate_program(CallableProgram::method(&method).unwrap(), &mut 100)
            .unwrap();
    }
}
