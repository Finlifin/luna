//! Borrow the ordinary telescope in its lexical checking frame. A pending
//! domain occupies a slot until a particular instance supplies its type.
use super::*;
use open_functions::Annotation;
use std::slice::Iter;

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
    pub(super) fn lexical_input_domains(&self) -> InputDomains<'_> {
        let owner = self.signature_owner;
        if let Some(slots) = owner
            .and_then(|owner| self.signature_domains.get(&owner))
            .or(self.default_input_scope.as_ref())
        {
            return InputDomains::Checking(slots.iter());
        }
        if let Some(instance) = self
            .open_instance
            .as_ref()
            .filter(|instance| Some(instance.owner) == owner)
        {
            let Term::Function { parameters, .. } = &instance.signature else {
                unreachable!("open instance has a function telescope")
            };
            return InputDomains::Formed(parameters.iter());
        }
        if let Some(definition) = owner.and_then(|owner| self.open_functions.get(&owner)) {
            return definition.input_domain_refs();
        }
        let signature = owner.and_then(|owner| {
            self.signatures
                .get(&owner)
                .or_else(|| self.interface_signatures.get(&owner))
        });
        match signature {
            Some(Term::Function { parameters, .. }) => InputDomains::Formed(parameters.iter()),
            None => InputDomains::Formed([].iter()),
            Some(_) => unreachable!("declaration has a function telescope"),
        }
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
