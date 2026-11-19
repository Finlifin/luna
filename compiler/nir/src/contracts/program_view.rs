//! Borrowed executable parts of a declaration or an interface contract.
//! Inspection does not specialize the contract or invent an executable body.
use crate::{
    DeclarationId, Term, TypeError,
    program::{
        EntrySnapshot, Expr, Function, FunctionDeclaration, FunctionPhase, LocalId, MethodContract,
        SourceRange,
    },
};
use std::fmt;

#[derive(Clone, Copy)]
pub struct CallableProgram<'a> {
    pub(crate) signature: &'a Term,
    pub(crate) parts: ProgramParts<'a>,
}

/// Executable roots and their lexical owner, independently of the input ABI.
/// Contextual method specifications have these parts without a callable Term.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct ProgramParts<'a> {
    pub(crate) declaration: Option<DeclarationId>,
    pub phase: FunctionPhase,
    pub(crate) is_unsafe: bool,
    pub type_parameters: &'a [DeclarationId],
    pub parameters: &'a [LocalId],
    pub body: Option<&'a Expr>,
    pub source: SourceRange,
    pub(crate) defaults: &'a [(usize, Expr)],
    pub preconditions: &'a [Expr],
    pub postconditions: &'a [Expr],
    pub result_binding: Option<LocalId>,
    pub entry_snapshots: &'a [EntrySnapshot],
}

impl<'a> CallableProgram<'a> {
    pub fn signature(&self) -> &'a Term {
        self.signature
    }

    pub fn parts(&self) -> &ProgramParts<'a> {
        &self.parts
    }

    /// Inspect each default alone in its lexical frame, before the call's
    /// preconditions hold. The enclosing body's roots and exit contracts
    /// belong to a different execution.
    pub fn default_bodies(self) -> impl Iterator<Item = Self> {
        self.parts.defaults.iter().map(move |(_, body)| {
            let mut program = Self::framed_body(
                body,
                self.signature,
                self.parts.phase,
                self.parts.parameters,
                self.parts.type_parameters,
            );
            program.parts.is_unsafe = self.parts.is_unsafe;
            program
        })
    }

    pub fn function(function: &'a Function) -> Self {
        let mut program = Self::declaration(&function.interface);
        program.parts.body = Some(&function.body);
        program
    }

    /// Inspect a specialized declaration without inventing executable code.
    pub fn declaration(function: &'a FunctionDeclaration) -> Self {
        Self {
            signature: &function.signature,
            parts: ProgramParts {
                declaration: Some(function.declaration),
                phase: function.phase,
                is_unsafe: function.is_unsafe,
                type_parameters: &function.type_parameters,
                parameters: &function.parameters,
                body: None,
                source: function.source,
                defaults: &function.defaults,
                preconditions: &function.preconditions,
                postconditions: &function.postconditions,
                result_binding: function.result_binding,
                entry_snapshots: &function.entry_snapshots,
            },
        }
    }

    /// Contextual code is not an ordinary callable. Its code interfaces are
    /// checked separately; this adapter cannot erase their category or ports.
    pub fn method(method: &'a MethodContract) -> Result<Self, TypeError> {
        Ok(Self::ordinary_method(method, method.signature.ordinary()?))
    }

    pub(crate) fn ordinary_method(method: &'a MethodContract, signature: &'a Term) -> Self {
        Self {
            signature: signature.callable_signature(),
            parts: ProgramParts::method(method),
        }
    }

    /// A comptime expression has no callable declaration or local arguments.
    /// An implementation template retains its lexical declaration parameters.
    pub fn comptime_body(
        body: &'a Expr,
        signature: &'a Term,
        type_parameters: &'a [DeclarationId],
    ) -> Self {
        Self::framed_body(
            body,
            signature,
            FunctionPhase::Comptime,
            &[],
            type_parameters,
        )
    }

    pub fn definition_region(region: &'a crate::program::DefinitionRegion) -> Self {
        Self::framed_body(
            &region.body,
            &region.signature,
            region.phase,
            &region.parameters,
            &region.type_parameters,
        )
    }

    fn framed_body(
        body: &'a Expr,
        signature: &'a Term,
        phase: FunctionPhase,
        parameters: &'a [LocalId],
        type_parameters: &'a [DeclarationId],
    ) -> Self {
        Self {
            signature,
            parts: ProgramParts {
                declaration: None,
                phase,
                is_unsafe: false,
                type_parameters,
                parameters,
                body: Some(body),
                source: body.source,
                defaults: &[],
                preconditions: &[],
                postconditions: &[],
                result_binding: None,
                entry_snapshots: &[],
            },
        }
    }

    /// Specialize only program specifications, without copying the executable
    /// body or defaults. The caller supplies the declaration substitution in
    /// the ordinary input frame; this does not prove either contract side.
    pub fn instantiate_contract(
        self,
        substitution: &crate::DeclarationSubstitution<'_>,
        fuel: &mut usize,
    ) -> Result<super::ProgramContract, crate::type_computations::Error> {
        super::instantiate_program_parts(
            self.parts.preconditions,
            self.parts.postconditions,
            self.parts.result_binding,
            self.parts.entry_snapshots,
            |value, fuel| substitution.instantiate_expression_bounded(value, fuel),
            fuel,
        )
    }

    /// Same root order as a Function traversal; consumers can reverse it to
    /// retain the existing work-stack order without collecting every root.
    pub fn roots(self) -> impl DoubleEndedIterator<Item = &'a Expr> {
        self.parts.roots()
    }
}

impl<'a> ProgramParts<'a> {
    pub(crate) fn method(method: &'a MethodContract) -> Self {
        Self {
            declaration: Some(method.declaration),
            phase: if method.signature.is_comptime_callable() {
                FunctionPhase::Comptime
            } else {
                FunctionPhase::Runtime
            },
            is_unsafe: method.is_unsafe,
            type_parameters: &method.type_parameters,
            parameters: &method.parameter_bindings,
            body: None,
            source: method.source,
            defaults: &[],
            preconditions: &method.program.preconditions,
            postconditions: &method.program.postconditions,
            result_binding: method.program.result_binding,
            entry_snapshots: &method.program.entry_snapshots,
        }
    }

    pub(crate) fn roots(self) -> impl DoubleEndedIterator<Item = &'a Expr> {
        self.body
            .into_iter()
            .chain(self.preconditions)
            .chain(self.postconditions)
            .chain(self.defaults.iter().map(|(_, value)| value))
    }
}

impl fmt::Display for CallableProgram<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(callable-program ")?;
        match self.parts.declaration {
            Some(declaration) => write!(f, "{declaration}")?,
            None => f.write_str("(anonymous)")?,
        }
        write!(
            f,
            " (phase {}) (unsafe {}) (signature {})",
            self.parts.phase, self.parts.is_unsafe, self.signature
        )?;
        f.write_str(" (type-parameters")?;
        for parameter in self.parts.type_parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (parameters")?;
        for parameter in self.parts.parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (requires")?;
        for condition in self.parts.preconditions {
            write!(f, " {condition}")?;
        }
        f.write_str(") (ensures")?;
        for condition in self.parts.postconditions {
            write!(f, " {condition}")?;
        }
        f.write_str(") (result-binding")?;
        if let Some(binding) = self.parts.result_binding {
            write!(f, " {binding}")?;
        }
        f.write_str(") (entry-snapshots")?;
        for snapshot in self.parts.entry_snapshots {
            write!(f, " {snapshot}")?;
        }
        f.write_str(") (defaults")?;
        for (index, value) in self.parts.defaults {
            write!(f, " ({index} {value})")?;
        }
        f.write_str(") (body")?;
        if let Some(body) = self.parts.body {
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
            source: flag(90).source,
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
            view.parts.parameters,
            method.parameter_bindings.as_slice()
        ));
        assert!(std::ptr::eq(
            view.parts.type_parameters,
            method.type_parameters.as_slice()
        ));
        assert!(std::ptr::eq(
            view.parts.entry_snapshots,
            method.program.entry_snapshots.as_slice()
        ));
        let roots: Vec<_> = view.roots().collect();
        assert_eq!(roots.len(), 2);
        assert!(std::ptr::eq(roots[0], &method.program.preconditions[0]));
        assert!(std::ptr::eq(roots[1], &method.program.postconditions[0]));
        assert!(view.parts.body.is_none());
        assert!(view.parts.defaults.is_empty());
        assert_eq!(view.parts.result_binding, method.program.result_binding);
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
    fn specialized_method_view_has_only_declaration_roots() {
        let declaration = method()
            .program_summary_declaration_inputs(&[], &[], &[], &[], &mut 1000)
            .unwrap();
        let view = CallableProgram::declaration(&declaration);
        assert!(view.parts.body.is_none());
        let roots: Vec<_> = view.roots().collect();
        assert_eq!(roots.len(), 2);
        assert!(std::ptr::eq(roots[0], &declaration.preconditions[0]));
        assert!(std::ptr::eq(roots[1], &declaration.postconditions[0]));
        assert!(
            declaration
                .to_string()
                .starts_with("(function-declaration ")
        );
        assert!(!declaration.to_string().contains("(body"));
    }

    #[test]
    fn detached_body_borrows_code_without_a_declaration_identity() {
        let body = flag(90);
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(body.ty.clone()),
        };
        let view = CallableProgram::comptime_body(&body, &signature, &[]);
        assert_eq!(view.parts.declaration, None);
        assert_eq!(view.parts.phase, FunctionPhase::Comptime);
        assert!(!view.parts.is_unsafe);
        assert!(std::ptr::eq(view.signature, &signature));
        let roots: Vec<_> = view.roots().collect();
        assert_eq!(roots.len(), 1);
        assert!(std::ptr::eq(roots[0], &body));
        assert!(
            view.to_string()
                .starts_with("(callable-program (anonymous) ")
        );
    }

    #[test]
    fn contract_specialization_does_not_copy_unrequested_body_or_defaults() {
        let interface = method()
            .program_summary_declaration_inputs(&[], &[], &[], &[], &mut 1000)
            .unwrap();
        let body = Expr {
            kind: K::Block {
                statements: (0..2000).map(flag).collect(),
                result: Some(Box::new(flag(2001))),
            },
            ..flag(0)
        };
        let mut function = Function { interface, body };
        function.defaults = vec![(0, function.body.clone())];
        let substitution =
            crate::DeclarationSubstitution::bounded(&[], &[], &[], &[], &mut 100).unwrap();
        let view = CallableProgram::function(&function);
        let contract = view.instantiate_contract(&substitution, &mut 100).unwrap();
        assert_eq!(contract.preconditions, function.preconditions);
        assert_eq!(contract.postconditions, function.postconditions);
        assert_eq!(contract.entry_snapshots, function.entry_snapshots);
        assert!(matches!(
            view.instantiate_contract(&substitution, &mut 0),
            Err(crate::type_computations::Error::Budget)
        ));
        assert_eq!(
            view.instantiate_contract(&substitution, &mut 100).unwrap(),
            contract
        );
        function.preconditions = (0..2000).map(flag).collect();
        assert!(matches!(
            CallableProgram::function(&function).instantiate_contract(&substitution, &mut 100),
            Err(crate::type_computations::Error::Budget)
        ));
        assert_eq!(
            CallableProgram::function(&function)
                .instantiate_contract(&substitution, &mut 100_000)
                .unwrap()
                .preconditions
                .len(),
            2000
        );
    }

    #[test]
    fn ordinary_program_roots_include_defaults_body_and_both_contract_sides() {
        let interface = method()
            .program_summary_declaration_inputs(&[], &[], &[], &[], &mut 1000)
            .unwrap();
        let mut function = Function {
            interface,
            body: flag(30),
        };
        function.defaults = vec![(0, flag(40))];
        let view = CallableProgram::function(&function);
        assert!(std::ptr::eq(view.parts.body.unwrap(), &function.body));
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
        assert_eq!(view.parts.phase, FunctionPhase::Comptime);
        crate::call_safety::validate_program(view, &mut 100).unwrap();
        // A scope in the postcondition cannot authorize a sibling precondition.
        method.program.preconditions = vec![call];
        let view = CallableProgram::method(&method).unwrap();
        assert!(
            crate::call_safety::validate_program(view, &mut 100)
                .unwrap_err()
                .to_string()
                .contains("71..72")
        );
        method.is_unsafe = true;
        crate::call_safety::validate_program(CallableProgram::method(&method).unwrap(), &mut 100)
            .unwrap();
    }
    #[test]
    fn call_safety_resource_failure_is_typed_and_recoverable() {
        let mut method = method();
        method.program.preconditions = (0..100).map(flag).collect();
        let view = CallableProgram::method(&method).unwrap();
        let mut fuel = 10;
        assert!(matches!(
            crate::call_safety::validate_program(view, &mut fuel),
            Err(crate::call_safety::Error::Budget(_))
        ));
        assert_eq!(fuel, 0);
        crate::call_safety::validate_program(view, &mut 101).unwrap();
    }
}
