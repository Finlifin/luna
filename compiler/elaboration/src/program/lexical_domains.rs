//! Borrow the ordinary telescope in its lexical checking frame. A pending
//! domain occupies a slot until a particular instance supplies its type.
use super::*;
use open_functions::Annotation;
use std::{slice::Iter, sync::Arc};

pub(super) type RetainedDomains = Arc<[(Sort, Option<Term>)]>;

enum DomainSource<'a> {
    Retained(&'a RetainedDomains),
    Borrowed(InputDomains<'a>),
}

impl<'a> DomainSource<'a> {
    fn domains(self) -> InputDomains<'a> {
        match self {
            Self::Retained(slots) => InputDomains::Checking(slots.iter()),
            Self::Borrowed(view) => view,
        }
    }

    fn retain(self, fuel: &mut u64, span: Span) -> Result<RetainedDomains, Error> {
        match self {
            Self::Retained(slots) => Ok(slots.clone()),
            Self::Borrowed(view) => {
                declaration_inputs::copy_domains(view, fuel, span).map(Into::into)
            }
        }
    }
}

pub(super) enum InputDomains<'a> {
    Checking(Iter<'a, (Sort, Option<Term>)>),
    Open(Iter<'a, (Sort, Annotation)>),
    Formed(Iter<'a, Parameter>),
}

impl<'a> Iterator for InputDomains<'a> {
    type Item = (Sort, Option<&'a Term>);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Checking(slots) => slots.next().map(|(sort, domain)| (*sort, domain.as_ref())),
            Self::Open(slots) => slots.next().map(|(sort, annotation)| {
                let domain = match annotation {
                    Annotation::Known(ty) => Some(ty),
                    _ => None,
                };
                (*sort, domain)
            }),
            Self::Formed(slots) => slots.next().map(|slot| (slot.sort, Some(&slot.ty))),
        }
    }
}

impl Checker<'_, '_> {
    fn lexical_domain_source(&self) -> DomainSource<'_> {
        let owner = self.signature_owner;
        if let Some(slots) = owner.and_then(|owner| self.signature_domains.get(&owner)) {
            return DomainSource::Borrowed(InputDomains::Checking(slots.iter()));
        }
        if let Some(slots) = &self.default_input_scope {
            return DomainSource::Retained(slots);
        }
        if let Some(instance) = self
            .open_instance
            .as_ref()
            .filter(|instance| Some(instance.owner) == owner)
        {
            let Term::Function { parameters, .. } = &instance.signature else {
                unreachable!("open instance has a function telescope")
            };
            return DomainSource::Borrowed(InputDomains::Formed(parameters.iter()));
        }
        if let Some(definition) = owner.and_then(|owner| self.open_functions.get(&owner)) {
            return DomainSource::Borrowed(definition.input_domain_refs());
        }
        let signature = owner.and_then(|owner| {
            self.signatures
                .get(&owner)
                .or_else(|| self.interface_signatures.get(&owner))
        });
        DomainSource::Borrowed(match signature {
            Some(Term::Function { parameters, .. }) => InputDomains::Formed(parameters.iter()),
            None => InputDomains::Formed([].iter()),
            Some(_) => unreachable!("declaration has a function telescope"),
        })
    }

    pub(super) fn lexical_input_domains(&self) -> InputDomains<'_> {
        self.lexical_domain_source().domains()
    }

    pub(super) fn retain_input_domains(&mut self, span: Span) -> Result<RetainedDomains, Error> {
        let mut fuel = self.required_steps;
        let retained = self.lexical_domain_source().retain(&mut fuel, span);
        self.required_steps = fuel;
        retained
    }

    pub(super) fn call_inference_domains(&self) -> Vec<(Sort, Option<Term>)> {
        self.lexical_input_domains()
            .map(|(sort, ty)| (sort, ty.cloned()))
            .collect()
    }

    pub(super) fn call_inference_scope(&self) -> Vec<Parameter> {
        self.lexical_input_domains()
            .map_while(|(sort, ty)| {
                Some(Parameter {
                    sort,
                    ty: ty?.clone(),
                })
            })
            .collect()
    }

    pub(super) fn call_input_frames(&self) -> Vec<Vec<Sort>> {
        let sorts: Vec<_> = self.lexical_input_domains().map(|(sort, _)| sort).collect();
        if sorts.is_empty() {
            vec![]
        } else {
            vec![sorts]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retaining_a_nested_default_scope_shares_the_original_telescope() {
        let slots = [
            (Sort::ComptimeValue, Some(Term::Tuple(vec![p(P::Str); 32]))),
            (Sort::Type, None),
        ];
        let saved = DomainSource::Borrowed(InputDomains::Checking(slots.iter()))
            .retain(&mut 1_000, Span::default())
            .unwrap();
        let nested = DomainSource::Retained(&saved)
            .retain(&mut 0, Span::default())
            .unwrap();
        assert!(Arc::ptr_eq(&saved, &nested));
        assert_eq!(nested.as_ref(), &slots);
    }
}
