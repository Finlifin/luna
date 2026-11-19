//! Capture the source telescope before looking up executable dependencies.
//! Bound types and parameter domains may select slots without a local read.
use super::*;
use std::borrow::Cow;

pub(super) struct Entry<'v> {
    pub(super) function: n::Function,
    pub(super) inputs: Vec<(n::LocalId, Term, Option<Term>)>,
    live_values: Vec<&'v Value>,
}

enum Supply<'v> {
    Frozen(Term),
    Live(&'v Value),
}

impl<'v> Entry<'v> {
    fn capture(
        root: n::Expr,
        mut slots: Vec<(n::LocalId, Sort, Option<Cow<'_, Term>>)>,
        mut supplies: BTreeMap<n::LocalId, Supply<'v>>,
        fuel: &mut usize,
    ) -> Result<Self, nir::type_computations::Error> {
        let captured = nir::type_computations::capture_entry_domains(
            DeclarationId {
                package: u64::MAX,
                local: u32::MAX,
            },
            "<required-comptime>".into(),
            root,
            slots
                .iter()
                .map(|(id, sort, domain)| (*id, *sort, domain.as_deref())),
            fuel,
        )?;
        let mut inputs = Vec::with_capacity(captured.slots.len());
        let mut live_values = Vec::new();
        for index in captured.slots {
            let (binding, _, domain) = &mut slots[index];
            let frozen = match supplies.remove(binding) {
                Some(Supply::Frozen(value)) => Some(value),
                Some(Supply::Live(value)) => {
                    live_values.push(value);
                    None
                }
                None => None,
            };
            let domain = match domain.take().expect("captured domain checked") {
                Cow::Owned(ty) => ty,
                Cow::Borrowed(ty) => nir::copy_budget::clone_term(ty, fuel)?,
            };
            inputs.push((*binding, domain, frozen));
        }
        Ok(Self {
            function: captured.function,
            inputs,
            live_values,
        })
    }

    pub(super) fn observe(
        &self,
        discovery: Discovery<'_>,
        steps: &mut u64,
        span: Span,
    ) -> Result<dependencies::ObservedInputs, Error> {
        let mut roots = discovery
            .function_references(&self.function, steps)
            .map_err(|failure| error(span, failure.to_string()))?;
        roots.extend(
            discovery.frozen_references(
                self.inputs
                    .iter()
                    .filter_map(|(_, _, value)| value.as_ref()),
                steps,
                span,
            )?,
        );
        let mut observed = dependencies::ObservedInputs {
            code: roots,
            ..Default::default()
        };
        for value in &self.live_values {
            observed.live(discovery, value, steps, span)?;
        }
        Ok(observed)
    }
}

fn staged_sort(domain: &Term) -> Sort {
    if *domain == p(P::Type) {
        Sort::Type
    } else {
        Sort::ComptimeValue
    }
}

impl Checker<'_, '_> {
    pub(super) fn capture_required_entry<'v>(
        &mut self,
        root: n::Expr,
        inputs: Vec<(n::LocalId, Term, Term)>,
        live: &'v [(n::LocalId, Term, Value)],
        span: Span,
    ) -> Result<Entry<'v>, Error> {
        let mut slots = Vec::new();
        let mut signature_slots = BTreeMap::new();
        if let Some(owner) = self
            .signature_owner
            .filter(|id| self.active_signatures.contains(id))
            && let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
                self.package.item(owner).map(|item| &item.kind)
        {
            for (parameter, (source_sort, domain)) in sig
                .params
                .iter()
                .zip(self.signature_domains.get(&owner).into_iter().flatten())
            {
                let binding = local(parameter.hir_id);
                let staged = sig.modifiers.is_comptime && *source_sort == Sort::Runtime;
                let sort = if staged {
                    domain.as_ref().map_or(Sort::ComptimeValue, staged_sort)
                } else {
                    *source_sort
                };
                signature_slots.insert(binding, (slots.len(), staged));
                slots.push((binding, sort, domain.as_ref().map(Cow::Borrowed)));
            }
        }

        let mut supplies = BTreeMap::new();
        for (binding, ty, value) in inputs {
            if let Some(&(index, staged)) = signature_slots.get(&binding) {
                if staged {
                    slots[index].1 = staged_sort(&ty);
                }
                slots[index].2 = Some(Cow::Owned(ty));
            } else {
                slots.push((binding, staged_sort(&ty), Some(Cow::Owned(ty))));
            }
            supplies.insert(binding, Supply::Frozen(value));
        }
        for (binding, ty, value) in live {
            if let Some(&(index, staged)) = signature_slots.get(binding) {
                if staged {
                    slots[index].1 = staged_sort(ty);
                }
                if let Some(expected) = &slots[index].2 {
                    self.require_expression(ty, expected, span)?;
                }
                slots[index].2 = Some(Cow::Borrowed(ty));
            } else {
                slots.push((*binding, staged_sort(ty), Some(Cow::Borrowed(ty))));
            }
            supplies.insert(*binding, Supply::Live(value));
        }

        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let capture = Entry::capture(root, slots, supplies, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        capture.map_err(|failure| {
            use nir::type_computations::Error::MissingCapture;
            let message = match failure {
                MissingCapture(binding) if self.required_runtime_parameter(binding) => {
                    "required comptime computation cannot read a runtime parameter; defer does not change the parameter's phase".into()
                }
                MissingCapture(binding) => format!("required comptime input {binding} is not available in this invocation"),
                other => other.to_string(),
            };
            error(span, message)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_cost_depends_on_the_selected_domains() {
        let boolean = p(P::Bool);
        let wide = Term::Tuple(vec![boolean.clone(); 2048]);
        let id = n::LocalId { owner: 1, index: 0 };
        let root = n::Expr {
            source: source(Span::default()),
            ty: boolean.clone(),
            kind: K::Literal(n::Literal::Bool(true)),
        };
        let capture = |root, domain, fuel: &mut usize| {
            Entry::capture(
                root,
                vec![(id, Sort::ComptimeValue, Some(Cow::Borrowed(domain)))],
                BTreeMap::new(),
                fuel,
            )
        };
        let mut narrow_fuel = 200;
        let mut wide_fuel = narrow_fuel;
        assert!(
            capture(root.clone(), &boolean, &mut narrow_fuel)
                .unwrap()
                .inputs
                .is_empty()
        );
        assert!(
            capture(root.clone(), &wide, &mut wide_fuel)
                .unwrap()
                .inputs
                .is_empty()
        );
        assert_eq!(wide_fuel, narrow_fuel);
        let read = n::Expr {
            ty: p(P::Type),
            kind: K::TypeValue(Term::NormalForm {
                head: DeclarationId {
                    package: 1,
                    local: 18,
                },
                sort: Sort::Type,
                arguments: vec![Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::ComptimeValue,
                }],
            }),
            ..root
        };
        assert!(matches!(
            capture(read.clone(), &wide, &mut 200),
            Err(nir::type_computations::Error::Budget)
        ));
        let entry = capture(read, &wide, &mut 100_000).unwrap();
        assert_eq!(entry.inputs, [(id, wide, None)]);
    }

    #[test]
    fn domain_only_captures_contribute_live_and_frozen_code_dependencies() {
        let id = |local| DeclarationId { package: 8, local };
        let binding = |index| n::LocalId { owner: 1, index };
        let choose = Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(p(P::Type)),
        }));
        let domain = Term::NormalForm {
            head: id(40),
            sort: Sort::Type,
            arguments: vec![Term::Bound {
                depth: 0,
                index: 1,
                sort: Sort::ComptimeValue,
            }],
        };
        // The Type expression mentions witness through a bound slot. Only
        // witness's domain mentions the callback. Neither is a local read.
        let root = n::Expr {
            source: source(Span::default()),
            ty: p(P::Type),
            kind: K::TypeValue(Term::NormalForm {
                head: id(50),
                sort: Sort::Type,
                arguments: vec![Term::Bound {
                    depth: 0,
                    index: 2,
                    sort: Sort::ComptimeValue,
                }],
            }),
        };
        let slots = vec![
            (binding(0), Sort::Runtime, None),
            (
                binding(1),
                Sort::ComptimeValue,
                Some(Cow::Owned(choose.clone())),
            ),
            (binding(2), Sort::ComptimeValue, Some(Cow::Owned(domain))),
            (binding(3), Sort::ComptimeValue, Some(Cow::Owned(choose))),
        ];
        assert!(
            nir::free_bindings::collect(&root, &mut 1000)
                .unwrap()
                .is_empty()
        );
        let target = Value::Function(id(41));
        let frozen = |id| Term::FunctionValue {
            declaration: id,
            arguments: vec![],
        };
        for callback in [Supply::Live(&target), Supply::Frozen(frozen(id(41)))] {
            let supplies = BTreeMap::from([
                (binding(1), callback),
                (binding(3), Supply::Frozen(frozen(id(42)))),
            ]);
            let captured =
                Entry::capture(root.clone(), slots.clone(), supplies, &mut 10000).unwrap();
            assert_eq!(captured.function.parameters, [binding(1), binding(2)]);
            assert_eq!(
                captured
                    .observe(Discovery::Declarations, &mut 1000, Span::default())
                    .unwrap()
                    .code,
                BTreeSet::from([Dependency::Callable(id(41))])
            );
        }
    }
}
