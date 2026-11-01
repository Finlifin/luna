//! Exact method positions in an immutable graph, never formation verdicts.
use super::*;

type Members = HashMap<DeclarationId, HashMap<Vec<Term>, usize>>;
type Subjects = HashMap<Term, (usize, Members)>;

#[derive(Default)]
pub(crate) struct MethodIndex(HashMap<DeclarationId, HashMap<Vec<Term>, Subjects>>);

impl MethodIndex {
    fn insert<'a>(
        &mut self,
        interface: DeclarationId,
        arguments: &[Term],
        subject: &Term,
        position: usize,
        methods: impl Iterator<Item = (DeclarationId, &'a [Term])>,
        fuel: &mut usize,
    ) -> Result<(), ()> {
        *fuel = fuel.checked_sub(1).ok_or(())?;
        for term in std::iter::once(subject).chain(arguments) {
            nir::type_computations::charge_term(term, fuel).map_err(|_| ())?;
        }
        let mut members: Members = HashMap::new();
        for (index, (member, arguments)) in methods.enumerate() {
            *fuel = fuel.checked_sub(1).ok_or(())?;
            for term in arguments {
                nir::type_computations::charge_term(term, fuel).map_err(|_| ())?;
            }
            members
                .entry(member)
                .or_default()
                .entry(arguments.to_vec())
                .or_insert(index);
        }
        self.0
            .entry(interface)
            .or_default()
            .entry(arguments.to_vec())
            .or_default()
            .entry(subject.clone())
            .or_insert((position, members));
        Ok(())
    }

    pub(super) fn contracts(graph: &iir::Package, fuel: &mut usize) -> Result<Self, ()> {
        let mut result = Self::default();
        for (index, contract) in graph.interfaces.iter().enumerate() {
            result.insert(
                contract.interface.declaration,
                &contract.interface.arguments,
                &contract.subject,
                index,
                contract
                    .methods
                    .iter()
                    .map(|m| (m.declaration, m.type_arguments.as_slice())),
                fuel,
            )?;
        }
        Ok(result)
    }

    pub(super) fn dispatch(graph: &iir::Package, fuel: &mut usize) -> Result<Self, ()> {
        let mut result = Self::default();
        for (index, dispatch) in graph.dispatch.iter().enumerate() {
            result.insert(
                dispatch.interface,
                &dispatch.interface_arguments,
                &dispatch.subject,
                index,
                dispatch
                    .methods
                    .iter()
                    .map(|m| (m.member, m.method_arguments.as_slice())),
                fuel,
            )?;
        }
        Ok(result)
    }

    /// Borrowed lookups allocate no key trees. The caller budgets traversal of
    /// input terms; unrelated interfaces and specializations are not visited.
    pub(crate) fn get(
        &self,
        interface: DeclarationId,
        arguments: &[Term],
        subject: &Term,
        member: DeclarationId,
        method_arguments: &[Term],
    ) -> Option<(usize, usize)> {
        let (outer, members) = self.0.get(&interface)?.get(arguments)?.get(subject)?;
        Some((*outer, *members.get(&member)?.get(method_arguments)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 93, local }
    }
    #[test]
    fn exact_keys_keep_all_specialization_axes_independent() {
        let mut index = MethodIndex::default();
        let mut fuel = 100_000;
        let terms = [
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::Char),
        ];
        let mut position = 0;
        for interface in [id(1), id(2)] {
            for argument in &terms {
                for subject in &terms {
                    index
                        .insert(
                            interface,
                            std::slice::from_ref(argument),
                            subject,
                            position,
                            [
                                (id(3), terms.as_slice()),
                                (id(4), terms.as_slice()),
                                (id(3), &terms[1..]),
                            ]
                            .into_iter(),
                            &mut fuel,
                        )
                        .unwrap();
                    position += 1;
                }
            }
        }
        position = 0;
        for interface in [id(1), id(2)] {
            for argument in &terms {
                for subject in &terms {
                    for (member, args, inner) in [
                        (id(3), terms.as_slice(), 0),
                        (id(4), terms.as_slice(), 1),
                        (id(3), &terms[1..], 2),
                    ] {
                        assert_eq!(
                            index.get(
                                interface,
                                std::slice::from_ref(argument),
                                subject,
                                member,
                                args
                            ),
                            Some((position, inner))
                        );
                    }
                    assert_eq!(
                        index.get(
                            interface,
                            std::slice::from_ref(argument),
                            subject,
                            id(3),
                            &[]
                        ),
                        None
                    );
                    position += 1;
                }
            }
        }
        assert_eq!(
            index.get(id(90), &terms[..1], &terms[0], id(3), &terms),
            None
        );
    }

    #[test]
    fn graph_index_construction_exhaustion_does_not_publish_partial_evidence() {
        let mut graph = iir::Package::default();
        for local in 0..128 {
            graph.dispatch.push(nir::program::DispatchInstance {
                interface: id(local),
                interface_arguments: vec![Term::Bool(true)],
                subject: Term::Primitive(Primitive::Bool),
                implementation: id(200 + local),
                arguments: vec![],
                methods: vec![nir::program::MethodInstance {
                    member: id(500),
                    method_arguments: vec![Term::Bool(false)],
                    body: id(600),
                    arguments: vec![],
                }],
                associated: vec![],
                associated_initializers: vec![],
            });
        }
        let baseline = graph.clone();
        let mut fuel = 100_000;
        MethodIndex::dispatch(&graph, &mut fuel).unwrap();
        let cost = 100_000 - fuel;
        for mut fuel in 0..cost {
            assert!(MethodIndex::dispatch(&graph, &mut fuel).is_err());
        }
        let index = MethodIndex::dispatch(&graph, &mut 100_000).unwrap();
        for local in 0..128 {
            assert_eq!(
                index.get(
                    id(local),
                    &[Term::Bool(true)],
                    &Term::Primitive(Primitive::Bool),
                    id(500),
                    &[Term::Bool(false)]
                ),
                Some((local as usize, 0))
            );
        }
        assert_eq!(graph, baseline);
    }
}
