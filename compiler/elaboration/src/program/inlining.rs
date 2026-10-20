//! Optional bounded expansion, including acyclic calls in expanded bodies.
//! Active declarations retain recursive calls rather than unfolding forever.
//! Resource checks and cleanup publication must run after this transformation.
use super::*;
use n::Rewriter;
mod effects;
pub(crate) use effects::expand_materialized as expand_materialized_effects;

/// Copy-local identities change; lexical inputs and transfers to an enclosing
/// loop keep their identities. Used when materialization duplicates a branch.
pub(crate) fn freshen_copied_expression(
    expression: &mut n::Expr,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut connections: BTreeMap<_, _> = nir::free_bindings::collect(expression, fuel)?
        .into_iter()
        .map(|id| (id, id))
        .collect();
    let mut loops = BTreeSet::new();
    nir::visit::expression(expression, fuel, |expr, fuel| {
        nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())?;
        if let K::Loop { identity, .. } = expr.kind {
            loops.insert(identity);
        }
        Ok(())
    })?;
    nir::visit::expression(expression, fuel, |expr, _| {
        if let K::Break(id) | K::Continue(id) = expr.kind {
            if !loops.contains(&id) {
                connections.insert(id, id);
            }
        }
        Ok(())
    })?;
    cmtt::freshen(
        expression,
        connections,
        next,
        sources,
        Span::new(
            rustc_span::BytePos(expression.source.start),
            rustc_span::BytePos(expression.source.end),
        ),
    )
    .map_err(|e| e.to_string())
}

/// Retry an ordinary inline call after its staged inputs have been bound.
/// The caller owns recursion detection and subsequent dependency normalization.
/// A failed candidate must not publish identities or return layouts.
pub(crate) fn expand_bound_inline(
    package: &n::Package,
    template: &n::Function,
    expression: &mut n::Expr,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    enums: &mut Vec<n::Enum>,
    fuel: &mut usize,
) -> Result<bool, String> {
    nir::visit::expression(expression, fuel, |expr, fuel| {
        nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
    })?;
    let mut candidate = expression.clone();
    let source_end = sources.len();
    let enum_end = enums.len();
    let old_next = *next;
    let result = cmtt::inline_template(
        Some(package),
        template,
        &mut candidate,
        vec![],
        next,
        sources,
        enums,
        &mut BTreeSet::new(),
        &mut BTreeMap::new(),
        fuel,
    );
    if result.is_ok() {
        *expression = candidate;
        Ok(true)
    } else {
        sources.truncate(source_end);
        enums.truncate(enum_end);
        *next = old_next;
        Ok(false)
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub expanded: usize,
    pub expanded_handlers: usize,
    pub expanded_requests: usize,
    pub retained: usize,
    pub selected_methods: usize,
    pub reused_method_selections: usize,
}
impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(inline-report (expanded {}) (retained {}) (selected-methods {}) (reused-method-selections {}) (expanded-handlers {}) (expanded-requests {}))",
            self.expanded,
            self.retained,
            self.selected_methods,
            self.reused_method_selections,
            self.expanded_handlers,
            self.expanded_requests
        )
    }
}

/// The input is typed NIR. Failed optional expansions leave the original call
/// and metadata intact. Staged calls await input completion; indirect calls and
/// open method selections are left to their existing invocation paths.
pub fn expand(package: &mut n::Package, fuel: &mut usize) -> Result<Report, String> {
    let mut templates = BTreeMap::new();
    for function in &package.functions {
        if !function.inline || function.phase != n::FunctionPhase::Runtime {
            continue;
        }
        nir::visit::expressions(function, fuel, |expr, fuel| {
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|error| error.to_string())
        })?;
        nir::type_computations::charge_term(&function.signature, fuel)
            .map_err(|e| e.to_string())?;
        templates.insert(function.declaration, function.clone());
    }
    // Selection observes the original checked package while expansion mutates
    // bodies and private return metadata. Charge the retained executable trees.
    let mut has_methods = false;
    for function in &package.functions {
        nir::visit::expressions(function, fuel, |expr, fuel| {
            has_methods |= matches!(expr.kind, K::TraitMethod { .. });
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
        })?;
    }
    // Associated annotations can require the same checked world even when
    // the expanded helper never invokes a trait method.
    let selection = if has_methods
        || package
            .traits
            .iter()
            .any(|item| !item.associated.is_empty())
    {
        for function in &package.quote_templates {
            nir::visit::expressions(function, fuel, |expr, fuel| {
                nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
            })?;
        }
        Some(package.clone())
    } else {
        None
    };
    let mut pass = Pass {
        selection: selection.as_ref(),
        methods: std::collections::HashMap::new(),
        active: BTreeSet::new(),
        templates: &templates,
        next: &mut package.quote_identity_cursor,
        sources: &mut package.expansion_sources,
        enums: &mut package.enums,
        packets: &mut package.internal_return_packets,
        empty: &mut package.internal_return_empty_replacements,
        fuel,
        report: Report::default(),
    };
    for function in &mut package.functions {
        if function.phase != n::FunctionPhase::Runtime {
            continue;
        }
        // Charge the traversal (including type clones performed by Rewriter)
        // before mutating this body. Exhaustion keeps all remaining bodies.
        if nir::visit::expression(&function.body, pass.fuel, |expr, fuel| {
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
        })
        .is_err()
        {
            break;
        }
        function.body.rewrite_with(&mut pass)?;
    }
    let mut report = pass.report;
    let (handlers, requests) = effects::expand(package, fuel)?;
    report.expanded_handlers = handlers;
    report.expanded_requests = requests;
    Ok(report)
}

// Representation equality is deliberately conservative: no opaque head is
// unfolded to make a cache hit. The immutable selection snapshot fixes all
// implementation and contract evidence for this pass.
#[derive(PartialEq, Eq, Hash)]
struct MethodKey {
    interface: DeclarationId,
    member: DeclarationId,
    subject: Term,
    interface_arguments: Vec<Term>,
    method_arguments: Vec<Term>,
}

struct Pass<'a> {
    active: BTreeSet<DeclarationId>,
    methods: std::collections::HashMap<MethodKey, n::MethodInstance>,
    selection: Option<&'a n::Package>,
    templates: &'a BTreeMap<DeclarationId, n::Function>,
    next: &'a mut u32,
    sources: &'a mut Vec<n::ExpansionSource>,
    enums: &'a mut Vec<n::Enum>,
    packets: &'a mut BTreeSet<DeclarationId>,
    empty: &'a mut BTreeMap<(u32, u32), DeclarationId>,
    fuel: &'a mut usize,
    report: Report,
}
impl n::Rewriter for Pass<'_> {
    type Error = String;
    fn ty(&mut self, ty: &Term) -> Result<Term, String> {
        Ok(ty.clone())
    }
    fn expression(&mut self, expression: &mut n::Expr) -> Result<bool, String> {
        if *self.fuel == 0 {
            return Ok(false);
        }
        *self.fuel -= 1;
        let K::Call { callee, .. } = &expression.kind else {
            return Ok(true);
        };
        let (id, types) = match &callee.kind {
            K::Function(id) => (*id, &[][..]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.as_slice()),
            K::TraitMethod {
                interface,
                member,
                subject,
                interface_arguments,
                method_arguments,
            } => {
                let Some(package) = self.selection else {
                    return Ok(true);
                };
                // Staged and still-open selections remain ordinary calls.
                if !matches!(callee.ty.callable_signature(), Term::Function {parameters, ..}
                    if parameters.iter().all(|parameter| parameter.sort == Sort::Runtime))
                {
                    return Ok(true);
                }
                // Bound hashing, key copies, and the rigid-input inspection.
                for ty in std::iter::once(subject)
                    .chain(interface_arguments)
                    .chain(method_arguments)
                {
                    if nir::type_computations::charge_term(ty, self.fuel).is_err() {
                        self.report.retained += 1;
                        return Ok(false);
                    }
                }
                let concrete = std::iter::once(subject)
                    .chain(interface_arguments)
                    .chain(method_arguments)
                    .all(|ty| {
                        ty.rigid_parameters()
                            .is_ok_and(|parameters| parameters.is_empty())
                    });
                if !concrete {
                    return Ok(true);
                }
                let key = MethodKey {
                    interface: *interface,
                    member: *member,
                    subject: subject.clone(),
                    interface_arguments: interface_arguments.clone(),
                    method_arguments: method_arguments.clone(),
                };
                let selected = match self.methods.entry(key) {
                    std::collections::hash_map::Entry::Occupied(slot) => {
                        self.report.reused_method_selections += 1;
                        slot.into_mut()
                    }
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        match crate::materialization::resolve_method_instance(
                            package, callee, self.fuel,
                        ) {
                            Ok(selected) => {
                                self.report.selected_methods += 1;
                                slot.insert(selected)
                            }
                            Err(_) => {
                                self.report.retained += 1;
                                return Ok(false);
                            }
                        }
                    }
                };
                (selected.body, selected.arguments.as_slice())
            }
            _ => return Ok(true),
        };
        let Some(template) = self.templates.get(&id) else {
            return Ok(true);
        };
        if self.active.contains(&id) || self.active.len() >= 64 {
            self.report.retained += 1;
            return Ok(false);
        }
        if !matches!(template.signature.callable_signature(),Term::Function {parameters,..}
            if parameters.iter().all(|parameter|parameter.sort==Sort::Runtime))
        {
            self.report.retained += 1;
            return Ok(true);
        }
        // Bound copying before attempting the transaction. Failure is an
        // optimization miss; the already checked ordinary call stays usable.
        let charged = nir::visit::expression(expression, self.fuel, |expr, fuel| {
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
        });
        if charged.is_err() {
            self.report.retained += 1;
            return Ok(false);
        }
        for ty in types {
            if nir::type_computations::charge_term(ty, self.fuel).is_err() {
                self.report.retained += 1;
                return Ok(false);
            }
        }
        let mut candidate = expression.clone();
        let types = types.to_vec();
        let source_end = self.sources.len();
        let enum_end = self.enums.len();
        let old_next = *self.next;
        let mut packets = BTreeSet::new();
        let mut empty = BTreeMap::new();
        let result = cmtt::inline_template(
            self.selection,
            template,
            &mut candidate,
            types,
            self.next,
            self.sources,
            self.enums,
            &mut packets,
            &mut empty,
            self.fuel,
        );
        if result.is_ok() {
            self.packets.extend(packets);
            self.empty.extend(empty);
            *expression = candidate;
            self.report.expanded += 1;
            // The outer expansion is already a valid committed replacement.
            // Further budget exhaustion retains its remaining ordinary calls.
            if nir::visit::expression(expression, self.fuel, |expr, fuel| {
                nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())
            })
            .is_ok()
            {
                self.active.insert(id);
                let nested = expression.rewrite_with(self);
                self.active.remove(&id);
                nested?;
            }
        } else {
            self.sources.truncate(source_end);
            self.enums.truncate(enum_end);
            *self.next = old_next;
            self.report.retained += 1;
        }
        Ok(false)
    }
}
