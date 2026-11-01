//! Positions and declaration relationships owned by one immutable IIR session.
//! No invocation values, mutable layouts or solver outcomes are retained here.
use super::*;

pub(crate) struct GraphIndexes {
    pub(crate) declarations: HashSet<DeclarationId>,
    pub(crate) nominals: HashMap<Term, usize>,
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
            let mut nominals = HashMap::new();
            for (index, instance) in graph.nominals.iter().enumerate() {
                fuel = fuel.checked_sub(1).ok_or(())?;
                // Pay for copying and hashing the full instance key once. The
                // immutable graph session shares this index across invocations.
                nir::type_computations::charge_term(&instance.ty, &mut fuel).map_err(|_| ())?;
                nominals.insert(instance.ty.clone(), index);
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
            let declarations = super::nominals::declaration_identities(graph, &mut fuel)?;
            Ok(Self {
                declarations,
                nominals,
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

impl<'a> Interpreter<'a> {
    /// Exact immutable instance lookup. Callers that already budget traversal of
    /// the type may use this directly; this stores no formation verdicts.
    pub(crate) fn concrete_nominal(&self, ty: &Term) -> Option<ConcreteNominal<'a>> {
        let graph = self.graph?;
        let indexes = self.graph_indexes.as_ref()?;
        if let Some(index) = indexes.nominals.get(ty) {
            return Some(ConcreteNominal::borrowed(
                &graph.nominals[*index].definition,
            ));
        }
        // Published batches are concrete definitions with their own identities.
        // Do not treat a missing generic instance as an empty-telescope type.
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return None;
        };
        if !arguments.is_empty() {
            return None;
        }
        if let Some(definition) = self.structs.get(declaration) {
            Some(ConcreteNominal::Struct(definition.clone()))
        } else if let Some(definition) = self.enums.get(declaration) {
            Some(ConcreteNominal::Enum(definition.clone()))
        } else {
            self.newtypes
                .handle(declaration)
                .map(ConcreteNominal::Newtype)
        }
    }

    pub(crate) fn concrete_nominal_metered(
        &mut self,
        ty: &Term,
    ) -> Result<Option<ConcreteNominal<'a>>, Error> {
        self.tick(None)?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = nir::type_computations::charge_term(ty, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
        Ok(self.concrete_nominal(ty))
    }
}

#[cfg(test)]
mod tests;
