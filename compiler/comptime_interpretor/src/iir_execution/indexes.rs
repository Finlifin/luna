//! Positions and declaration relationships owned by one immutable IIR session.
//! No invocation values, mutable layouts or solver outcomes are retained here.
use super::*;

pub(crate) struct GraphIndexes {
    pub(crate) callables: HashMap<DeclarationId, HashMap<Arc<[Term]>, usize>>,
    pub(crate) normal_forms: Arc<BTreeMap<DeclarationId, usize>>,
    pub(crate) implementations: Arc<HashMap<(InterfaceInstance, Term), DeclarationId>>,
}
impl GraphIndexes {
    pub(crate) fn new(graph: &iir::Package, budget: &mut Budget) -> Result<Self, Error> {
        let before = usize::try_from(budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = (|| {
            let mut callables: HashMap<_, HashMap<_, _>> = HashMap::new();
            for (index, identity) in graph.callable_identities.iter().enumerate() {
                fuel = fuel.checked_sub(1).ok_or(())?;
                for argument in identity.key.arguments.iter() {
                    nir::type_computations::charge_term(argument, &mut fuel).map_err(|_| ())?;
                }
                // The graph owns immutable argument slices. Share their owner
                // rather than copying every term into a second key tree.
                callables
                    .entry(identity.key.declaration)
                    .or_default()
                    .insert(identity.key.arguments.clone(), index);
            }
            let mut normal_forms = BTreeMap::new();
            for (index, head) in graph.normal_forms.iter().enumerate() {
                fuel = fuel.checked_sub(1).ok_or(())?;
                normal_forms.insert(head.declaration, index);
            }
            let mut implementations = HashMap::new();
            for dispatch in &graph.dispatch {
                fuel = fuel.checked_sub(1).ok_or(())?;
                nir::type_computations::charge_term(&dispatch.subject, &mut fuel)
                    .map_err(|_| ())?;
                for argument in &dispatch.interface_arguments {
                    nir::type_computations::charge_term(argument, &mut fuel).map_err(|_| ())?;
                }
                implementations.insert(
                    (dispatch.interface_instance(), dispatch.subject.clone()),
                    dispatch.implementation,
                );
            }
            Ok(Self {
                callables,
                normal_forms: Arc::new(normal_forms),
                implementations: Arc::new(implementations),
            })
        })();
        budget.steps -= (before - fuel) as u64;
        result.map_err(|()| Error {
            kind: ErrorKind::StepsExhausted,
            source: None,
        })
    }
}
