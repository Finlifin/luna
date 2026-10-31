//! One prepared specialization per immutable Invocation owner. This retains
//! code, never ordinary argument values, results, or discharged contracts.
use super::*;
use std::sync::Arc;

pub(super) struct Cache {
    pub enabled: bool,
    slots: Vec<usize>,
    entry: Option<Entry>,
    pub code: Option<comptime_interpretor::CodeSession>,
    pub admission: Option<comptime_interpretor::CodeSession>,
    pub preparations: usize,
    pub stats: crate::invocation::InvocationStats,
}
struct Entry {
    inputs: Vec<Term>,
    package: Arc<Package>,
    graph: Arc<iir::Package>,
    declaration: DeclarationId,
}
pub(super) type Prepared = (Arc<Package>, Arc<iir::Package>, DeclarationId);
impl Cache {
    pub fn new(enabled: bool, signature: &Term) -> Self {
        let Term::Function { parameters, .. } = signature.callable_signature() else {
            unreachable!("validated invocation signature")
        };
        Self {
            enabled,
            slots: parameters
                .iter()
                .enumerate()
                .filter_map(|(i, p)| (p.sort == nir::Sort::Type).then_some(i))
                .collect(),
            entry: None,
            code: None,
            admission: None,
            preparations: 0,
            stats: Default::default(),
        }
    }
    pub fn prepare(
        &mut self,
        package: &Package,
        declaration: DeclarationId,
        arguments: &[Term],
        values: Vec<Value>,
        budget: ptc::Budget,
        context: &mut Context,
    ) -> Result<(Option<Prepared>, Vec<Value>), String> {
        if !self.enabled {
            return Ok((None, values));
        }
        let mut inputs = Vec::with_capacity(self.slots.len());
        for &slot in &self.slots {
            let Some(Value::Type(ty)) = values.get(slot) else {
                return Err("staged Type input is not Type".into());
            };
            let before = context.remaining();
            let mut fuel = before;
            let charged = nir::type_computations::charge_term(ty, &mut fuel);
            context
                .spend::<String>(before - fuel)
                .map_err(|e| e.to_string())?;
            charged.map_err(|e| e.to_string())?;
            inputs.push(ty.clone());
        }
        // Meter both sides before structural equality. This is a conservative
        // code key, not source Eq/Hash or a claim of logical type inequality.
        if let Some(entry) = &self.entry {
            for ty in &entry.inputs {
                let before = context.remaining();
                let mut fuel = before;
                let charged = nir::type_computations::charge_term(ty, &mut fuel);
                context
                    .spend::<String>(before - fuel)
                    .map_err(|e| e.to_string())?;
                charged.map_err(|e| e.to_string())?;
            }
            if entry.inputs == inputs {
                context
                    .spend::<String>(values.len())
                    .map_err(|e| e.to_string())?;
                let mut slots = self.slots.iter().copied().peekable();
                let retained = values
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, value)| {
                        if slots.peek() == Some(&i) {
                            slots.next();
                            None
                        } else {
                            Some(value)
                        }
                    })
                    .collect();
                return Ok((
                    Some((
                        entry.package.clone(),
                        entry.graph.clone(),
                        entry.declaration,
                    )),
                    retained,
                ));
            }
        }
        let (prepared, graph, entry, type_arguments, retained) =
            prepare_bound_entry(package, declaration, arguments, values, budget, context)?;
        let graph = Arc::new(graph.ok_or("missing staged invocation graph")?);
        if !type_arguments.is_empty() {
            return Err("staged invocation retains unbound declaration inputs".into());
        }
        let package = Arc::new(prepared.into_owned());
        // Publish only after preparation succeeds. A failed attempt retains the
        // prior specialization; a successful different key replaces it.
        self.code = Some(comptime_interpretor::CodeSession::for_iir(graph.clone()));
        self.admission = Some(comptime_interpretor::CodeSession::new(package.clone()));
        self.entry = Some(Entry {
            inputs,
            package: package.clone(),
            graph: graph.clone(),
            declaration: entry,
        });
        self.preparations += 1;
        Ok((Some((package, graph, entry)), retained))
    }
}
