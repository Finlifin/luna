//! Invocation-local binding handoff for completed code fragments. Frames share their lexical
//! export layout with the code and publish values only on normal completion.
use super::*;

/// Indices into the program's immutable local layout. Consecutive imports
/// starting at zero need no per-binding storage. Later holes distinguish
/// caller-owned bindings from locals created by the running code.
#[derive(Default)]
pub(super) struct Imports {
    prefix: usize,
    rest: Vec<usize>,
}

impl Imports {
    pub(super) fn record(&mut self, index: usize) {
        if index == self.prefix {
            self.prefix += 1;
        } else {
            self.rest.push(index);
        }
    }

    pub(super) fn bindings<'a>(
        &'a self,
        code: &'a Program,
    ) -> impl Iterator<Item = (LocalId, usize)> + 'a {
        (0..self.prefix)
            .chain(self.rest.iter().copied())
            .map(|index| code.locals()[index])
    }
}

impl Frame {
    pub(super) fn complete_environment(
        mut self,
        mut flow: Result<Flow, Error>,
    ) -> (Result<Flow, Error>, Environment) {
        if let Some(exports) = self.exports.take() {
            if matches!(&flow, Ok(Flow::Value(_))) {
                let invalid = if !self.terminal || self.waiting.is_some() || !self.scopes.is_empty()
                {
                    Some(ErrorKind::InvalidControlFlow)
                } else if exports
                    .bindings()
                    .iter()
                    .any(|(binding, _)| self.environment.contains_key(binding))
                {
                    Some(ErrorKind::Type)
                } else if exports
                    .bindings()
                    .iter()
                    .any(|(_, slot)| self.slots.get(*slot).and_then(Option::as_ref).is_none())
                {
                    Some(ErrorKind::MissingBinding)
                } else {
                    None
                };
                if let Some(kind) = invalid {
                    flow = Err(Error {
                        kind,
                        source: Some(exports.source()),
                    });
                } else {
                    // Validate the whole handoff before making any new binding
                    // visible. Values move out of slots, preserving storage and
                    // value-reference identity; no source Copy/Clone is invoked.
                    for &(binding, slot) in exports.bindings() {
                        let value = self.slots[slot].take().expect("validated export");
                        self.environment.insert(binding, value);
                    }
                }
            }
        }
        // Errors and non-local control still return original inputs with their
        // actual mutations. They never publish partially initialized exports.
        (flow, self.restore_environment())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fragment_restores_only_imports_across_gaps_in_its_local_layout() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let local = |index| LocalId { owner: 3, index };
        let boolean = Term::Primitive(Primitive::Bool);
        let void = Term::Primitive(Primitive::Void);
        let expr = |ty, kind| Expr {
            source: SourceRange { start: 1, end: 2 },
            ty,
            kind,
        };
        let value = || Box::new(expr(boolean.clone(), K::Literal(Literal::Bool(false))));
        let assign = |root| {
            expr(
                void.clone(),
                K::Assign {
                    place: nir::program::Place {
                        root,
                        fields: vec![],
                    },
                    value: value(),
                },
            )
        };
        for first in [0, 2] {
            let input = local(first);
            let temporary = local(1);
            let nested = local(4);
            let untouched = local(30);
            let body = expr(
                boolean.clone(),
                K::Block {
                    statements: vec![
                        expr(
                            void.clone(),
                            K::Let {
                                binding: temporary,
                                value: value(),
                            },
                        ),
                        assign(input),
                        assign(nested),
                    ],
                    result: Some(Box::new(expr(boolean.clone(), K::Local(nested)))),
                },
            );
            let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
            let mut env = Environment::from([(input, Value::Bool(true)), (untouched, Value::Unit)]);
            env.begin_scope();
            env.insert(nested, Value::Bool(true));
            assert!(matches!(
                vm.execute_code(&code, &mut env),
                Ok(Flow::Value(Value::Bool(false)))
            ));
            assert_eq!(env.get(&input), Some(&Value::Bool(false)));
            assert_eq!(env.get(&nested), Some(&Value::Bool(false)));
            assert!(!env.contains_key(&temporary));
            env.leave_scope();
            assert_eq!(
                env,
                Environment::from([(input, Value::Bool(false)), (untouched, Value::Unit)])
            );
        }
    }
}
