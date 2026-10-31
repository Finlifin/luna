//! Bounded specialization of immutable default code, never invocation results.
use super::*;
use std::sync::Weak;

#[derive(PartialEq)]
enum Key {
    Interface(Vec<nir::DeclarationParameter>, Vec<Term>),
    Function(DeclarationId, Vec<(DeclarationId, Term)>, Vec<Term>),
}
struct Entry {
    source: Weak<Expr>,
    key: Key,
    key_work: u64,
    expression: Arc<Expr>,
    weight: u64,
}
#[derive(Default)]
pub(super) struct Cache {
    entries: Vec<Entry>,
    weight: u64,
}
impl Cache {
    fn lookup(
        &mut self,
        source: &Arc<Expr>,
        key: &Key,
        fuel: &mut u64,
    ) -> Result<Option<Arc<Expr>>, ()> {
        *fuel = fuel.checked_sub(self.entries.len() as u64).ok_or(())?;
        self.entries
            .retain(|entry| entry.source.strong_count() != 0);
        self.weight = self.entries.iter().map(|entry| entry.weight).sum();
        for entry in &self.entries {
            if entry
                .source
                .upgrade()
                .is_some_and(|body| Arc::ptr_eq(&body, source))
            {
                *fuel = fuel.checked_sub(entry.key_work).ok_or(())?;
                if &entry.key == key {
                    return Ok(Some(entry.expression.clone()));
                }
            }
        }
        Ok(None)
    }
    fn insert(
        &mut self,
        source: &Arc<Expr>,
        key: Key,
        key_work: u64,
        expression: Arc<Expr>,
        work: u64,
    ) {
        const LIMIT: u64 = 100_000;
        let weight = work.saturating_add(key_work).saturating_add(1);
        if weight > LIMIT {
            return;
        }
        if self.entries.len() >= 64 || self.weight.saturating_add(weight) > LIMIT {
            self.entries.clear();
            self.weight = 0;
        }
        self.weight += weight;
        self.entries.push(Entry {
            source: Arc::downgrade(source),
            key,
            key_work,
            expression,
            weight,
        });
    }
}

// Charge owned key construction before cloning any recursive input. Function
// keys include the declaration domains used by dependent substitution.
fn charge_key<'a>(
    terms: impl IntoIterator<Item = &'a Term>,
    count: usize,
    fuel: &mut u64,
) -> Result<u64, nir::copy_budget::CopyBudgetExhausted> {
    let before = usize::try_from(*fuel).unwrap_or(usize::MAX);
    let mut remaining = before;
    let checked = (|| {
        remaining = remaining
            .checked_sub(count.saturating_add(1))
            .ok_or(nir::copy_budget::CopyBudgetExhausted::Steps)?;
        for term in terms {
            nir::copy_budget::charge_type_substitution(term, &[], &[], &mut remaining)?;
        }
        Ok(())
    })();
    let work = (before - remaining) as u64;
    *fuel -= work;
    checked.map(|()| work)
}

impl Interpreter<'_> {
    pub(super) fn specialized_default_code(
        &mut self,
        source: &Arc<Expr>,
        interface: Option<&(Vec<nir::DeclarationParameter>, Vec<Term>)>,
        dynamic: bool,
        callee: &Value,
        construction: SourceRange,
    ) -> Result<Arc<Expr>, Error> {
        let key_error = |reason| Error {
            kind: match reason {
                nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
                nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
            },
            source: Some(source.source),
        };
        let (key, key_work) = if let Some((binders, arguments)) = interface {
            let work = charge_key(
                arguments.iter(),
                binders.len() + arguments.len(),
                &mut self.budget.steps,
            )
            .map_err(key_error)?;
            (Key::Interface(binders.clone(), arguments.clone()), work)
        } else if dynamic
            && let Value::Closure(closure) = callee
            && let Some((declaration, arguments)) = closure.declaration_instance()
        {
            let definition = self
                .functions
                .get(&declaration)
                .copied()
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(construction)))?;
            let type_domain = Term::Primitive(Primitive::Type);
            let domains = definition.type_parameters.iter().map(|parameter| {
                self.declaration_parameter_domains
                    .get(parameter)
                    .unwrap_or(&type_domain)
            });
            let work = charge_key(
                domains.chain(arguments.iter()),
                definition.type_parameters.len() + arguments.len(),
                &mut self.budget.steps,
            )
            .map_err(key_error)?;
            let domains = definition
                .type_parameters
                .iter()
                .map(|parameter| {
                    (
                        *parameter,
                        self.declaration_parameter_domains
                            .get(parameter)
                            .unwrap_or(&type_domain)
                            .clone(),
                    )
                })
                .collect();
            (
                Key::Function(declaration, domains, arguments.to_vec()),
                work,
            )
        } else {
            return Ok(source.clone());
        };
        if let Some(expression) = self
            .default_code_cache
            .lookup(source, &key, &mut self.budget.steps)
            .map_err(|()| Error {
                kind: ErrorKind::StepsExhausted,
                source: Some(source.source),
            })?
        {
            return Ok(expression);
        }
        let before = self.budget.steps;
        let expression = Arc::new(if let Some((binders, arguments)) = interface {
            self.bound_default_expression(source, binders, arguments)?
        } else {
            self.instance_default_expression(construction, source, callee)?
        });
        let work = before - self.budget.steps;
        self.default_code_cache
            .insert(source, key, key_work, expression.clone(), work);
        Ok(expression)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn template() -> Arc<Expr> {
        Arc::new(Expr {
            source: SourceRange { start: 3, end: 4 },
            ty: Term::Primitive(Primitive::Type),
            kind: K::TypeValue(Term::Rigid {
                declaration: DeclarationId {
                    package: 1,
                    local: 1,
                },
                sort: nir::Sort::Type,
            }),
        })
    }
    fn inputs(ty: Primitive) -> (Vec<nir::DeclarationParameter>, Vec<Term>) {
        (
            vec![nir::DeclarationParameter {
                declaration: DeclarationId {
                    package: 1,
                    local: 1,
                },
                sort: nir::Sort::Type,
            }],
            vec![Term::Primitive(ty)],
        )
    }
    #[test]
    fn specialization_reuses_code_but_separates_templates_and_inputs() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let source = template();
        let boolean = inputs(Primitive::Bool);
        let character = inputs(Primitive::Char);
        let first = vm
            .specialized_default_code(&source, Some(&boolean), false, &Value::Unit, source.source)
            .unwrap();
        let second = vm
            .specialized_default_code(
                &source,
                Some(&character),
                false,
                &Value::Unit,
                source.source,
            )
            .unwrap();
        assert!(matches!(
            first.kind,
            K::TypeValue(Term::Primitive(Primitive::Bool))
        ));
        assert!(matches!(
            second.kind,
            K::TypeValue(Term::Primitive(Primitive::Char))
        ));
        assert!(!Arc::ptr_eq(&first, &second));
        let reused = vm
            .specialized_default_code(&source, Some(&boolean), false, &Value::Unit, source.source)
            .unwrap();
        assert!(Arc::ptr_eq(&first, &reused));
        let other = template();
        let distinct = vm
            .specialized_default_code(&other, Some(&boolean), false, &Value::Unit, other.source)
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &distinct));
        let weak = Arc::downgrade(&source);
        drop(source);
        assert!(weak.upgrade().is_none());
        vm.specialized_default_code(&other, Some(&boolean), false, &Value::Unit, other.source)
            .unwrap();
        assert_eq!(vm.default_code_cache.entries.len(), 1);
    }
    #[test]
    fn failed_specialization_and_budget_exhaustion_do_not_publish() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let source = template();
        let mut wrong = inputs(Primitive::Bool);
        wrong.1.clear();
        assert!(
            vm.specialized_default_code(&source, Some(&wrong), false, &Value::Unit, source.source)
                .is_err()
        );
        assert!(vm.default_code_cache.entries.is_empty());
        let correct = inputs(Primitive::Bool);
        vm.budget.steps = 0;
        assert_eq!(
            vm.specialized_default_code(
                &source,
                Some(&correct),
                false,
                &Value::Unit,
                source.source
            )
            .unwrap_err()
            .kind,
            ErrorKind::StepsExhausted
        );
        assert!(vm.default_code_cache.entries.is_empty());
        vm.budget.steps = 10000;
        let formed = vm
            .specialized_default_code(&source, Some(&correct), false, &Value::Unit, source.source)
            .unwrap();
        vm.budget.steps = 0;
        assert_eq!(
            vm.specialized_default_code(
                &source,
                Some(&correct),
                false,
                &Value::Unit,
                source.source
            )
            .unwrap_err()
            .kind,
            ErrorKind::StepsExhausted
        );
        vm.budget.steps = 10000;
        let recovered = vm
            .specialized_default_code(&source, Some(&correct), false, &Value::Unit, source.source)
            .unwrap();
        assert!(Arc::ptr_eq(&formed, &recovered));
    }
    #[test]
    fn capacity_eviction_does_not_retain_unbounded_code() {
        let source = template();
        let mut cache = Cache::default();
        for index in 0..200 {
            cache.insert(
                &source,
                Key::Interface(vec![], vec![Term::Bool(index % 2 == 0)]),
                2,
                template(),
                2000,
            );
            assert!(cache.entries.len() <= 64);
            assert!(cache.weight <= 100000);
        }
        let count = cache.entries.len();
        cache.insert(
            &source,
            Key::Interface(vec![], vec![]),
            2,
            template(),
            100001,
        );
        assert_eq!(cache.entries.len(), count);
    }
}
