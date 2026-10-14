//! Bounded program-behavior analysis, independent of PTC and interpretation.
//! Runtime `pure` promises require P4, while runtime defaults also require finite
//! completion. Recursion/loops alone do not violate P4; neither mode authorizes
//! unknown calls, external storage effects or unaccounted resource cleanup.
use super::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CodeKey { Named(DeclarationId, Vec<Term>), Closure(usize) }
type Inputs = BTreeMap<usize, Vec<CodeKey>>;
type Codes = Vec<n::Expr>;
type Key = (DeclarationId, Vec<Term>, Inputs);

pub(super) fn validate_runtime_promises(package: &n::Package, protocols: &Protocols) -> Result<(), Error> {
    let mut analysis = Analysis::new(package, false);
    analysis.protocols = protocols.clone();
    for function in &package.functions {
        if function.phase != n::FunctionPhase::Runtime || !function.pure { continue; }
        let types = function.type_parameters.iter().map(|declaration|
            Term::Rigid { declaration: *declaration, sort: Sort::Type }).collect::<Vec<_>>();
        if !analysis.function(function.declaration, &types, 0) {
            return Err(Error {
                source: function.body.source,
                message: format!("runtime pure function `{}` lacks P4 evidence for its calls, storage or resource behavior", function.name),
                needs_context: false, needs_tactic: false, needs_capability: false, incompatible_context: false,
            });
        }
    }
    Ok(())
}

pub(super) fn validate_defaults(package: &n::Package, obligations: &[(n::Expr, Span)]) -> Result<(), Error> {
    let mut analysis = Analysis::new(package, true);
    for (value, span) in obligations {
        if !analysis.expression(value, &mut BTreeSet::new(), false, 0) {
            return Err(error(*span,
                "default expression requires P3 stability/termination evidence; call graph or operations could not be certified"));
        }
    }
    Ok(())
}

struct Analysis<'a> {
    package: &'a n::Package,
    protocols: Protocols,
    functions: HashMap<DeclarationId, &'a n::Function>,
    tuple_heads: HashSet<DeclarationId>,
    finite: bool,
    callables: BTreeMap<n::LocalId, Codes>,
    closures: Vec<n::Expr>,
    result_code: Option<Codes>,
    active: HashSet<Key>,
    proven: HashMap<Key, bool>,
    fuel: usize,
}

impl<'a> Analysis<'a> {
    fn new(package: &'a n::Package, finite: bool) -> Self {
        Self {
            package, protocols: Protocols::default(),
            functions: package.functions.iter().map(|f| (f.declaration, f)).collect(),
            tuple_heads: package.normal_forms.iter().filter(|head| head.role == nir::normal_forms::Role::Tuple)
                .map(|head| head.declaration).collect(),
            finite, callables: BTreeMap::new(), closures: vec![], result_code: None, active: HashSet::new(), proven: HashMap::new(), fuel: 100_000,
        }
    }

    fn copy_type(&self, ty: &Term) -> bool {
        fn inspect(ty: &Term, heads: &HashSet<DeclarationId>, depth: usize) -> bool {
            if depth >= 128 { return false; }
            match ty {
                Term::Tuple(fields) => fields.iter().all(|ty| inspect(ty, heads, depth + 1)),
                Term::Record(fields) => fields.iter().all(|(_, ty)| inspect(ty, heads, depth + 1)),
                Term::NormalForm { head, arguments, .. } if heads.contains(head) => {
                    matches!(arguments.as_slice(), [Term::TypeListValue(fields)]
                        if fields.iter().all(|ty| inspect(ty, heads, depth + 1)))
                }
                _ => copy_scalar(ty),
            }
        }
        inspect(ty, &self.tuple_heads, 0)
    }

    fn expression(&mut self, value: &n::Expr, private: &mut BTreeSet<n::LocalId>, returns: bool, depth: usize) -> bool {
        self.result_code = None;
        let valid = self.check_expression(value, private, returns, depth);
        let returned = self.result_code.take();
        self.result_code = if valid { match &value.kind {
            K::Function(_) | K::FunctionInstance { .. } => Some(vec![value.clone()]),
            K::Closure { captures, .. } if self.supported_captures(captures) => Some(vec![value.clone()]),
            K::Local(binding) => self.callables.get(binding).cloned(),
            K::Call { .. } | K::If { .. } | K::Match { .. } | K::Block { .. } | K::CallPreparation { .. } => returned,
            _ => None,
        } } else { None };
        valid
    }

    fn check_expression(&mut self, value: &n::Expr, private: &mut BTreeSet<n::LocalId>, returns: bool, depth: usize) -> bool {
        if depth >= 128 || self.fuel == 0 { return false; }
        self.fuel -= 1;
        match &value.kind {
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let saved = private.clone();
                let visible_code = self.callables.keys().copied().collect::<BTreeSet<_>>();
                let valid = statements.iter().all(|s| self.expression(s, private, returns, depth + 1))
                    && result.as_ref().is_none_or(|r| self.expression(r, private, returns, depth + 1));
                if result.is_none() { self.result_code = None; }
                *private = saved;
                self.callables.retain(|binding, _| visible_code.contains(binding));
                valid
            }
            K::Let { binding, value } => {
                if (self.finite && !self.copy_type(&value.ty)) || !self.expression(value, private, returns, depth + 1) { return false; }
                if let Some(code) = self.result_code.clone() { self.callables.insert(*binding, code); }
                private.insert(*binding);
                true
            }
            K::LetPattern { pattern, value, guard, otherwise } => {
                otherwise.is_none() && stable_pattern(pattern) && (!self.finite || self.copy_type(&value.ty))
                    && self.expression(value, private, returns, depth + 1)
                    && guard.as_ref().is_none_or(|guard| self.copy_type(&guard.ty)
                        && self.expression(guard, private, returns, depth + 1))
            }
            K::Local(_) => !self.finite || self.copy_type(&value.ty),
            K::Literal(_) | K::Unit | K::AssociatedValue { .. } => self.copy_type(&value.ty),
            K::Assign { place, value } => {
                if !private.contains(&place.root) || (self.finite && !self.copy_type(&value.ty))
                    || !self.expression(value, private, returns, depth + 1) { return false; }
                // Widen rather than choosing the last syntactically visited
                // branch. Unknown is sticky, including across loop backedges.
                let previous = self.callables.remove(&place.root);
                let replacement = self.result_code.take();
                if let Some(codes) = self.join_codes(previous, replacement) {
                    self.callables.insert(place.root, codes);
                }
                true
            }
            K::BitAssign { place, storage_type, value, .. } => {
                private.contains(&place.root) && self.copy_type(storage_type)
                    && self.expression(value, private, returns, depth + 1)
            }
            K::BitUpdate { place, storage_type, selections, value } => {
                // Positions are computations too. Inspect every nested window
                // in preparation order before the payload. Bounds and storage
                // validity remain obligations of the program verification pass.
                private.contains(&place.root) && self.copy_type(storage_type)
                    && selections.iter().flat_map(n::BitSelection::expressions)
                        .all(|input| self.expression(input, private, returns, depth + 1))
                    && self.expression(value, private, returns, depth + 1)
            }
            K::BitWindow { value: inner, start, end } => {
                (self.copy_type(&value.ty) || is_never(&value.ty))
                    && self.expression(inner, private, returns, depth + 1)
                    && self.expression(start, private, returns, depth + 1)
                    && self.expression(end, private, returns, depth + 1)
            }
            K::BitIndex(inner, index) => {
                (self.copy_type(&value.ty) || is_never(&value.ty))
                    && self.expression(inner, private, returns, depth + 1)
                    && self.expression(index, private, returns, depth + 1)
            }
            K::Loop { body, .. } => {
                if self.finite || !self.loop_code_closure(body) { return false; }
                self.expression(body, &mut private.clone(), returns, depth + 1)
            },
            K::Break(_) | K::Continue(_) => !self.finite,
            K::Return(value) => returns && value.as_ref().is_none_or(|value|
                (!self.finite || self.copy_type(&value.ty)) && self.expression(value, private, returns, depth + 1)),
            K::If { condition, then_branch, else_branch } => {
                if !self.copy_type(&condition.ty) || !self.expression(condition, private, returns, depth + 1)
                    || !self.expression(then_branch, &mut private.clone(), returns, depth + 1) { return false; }
                let left = self.result_code.take();
                let Some(branch) = else_branch else { return true; };
                if !self.expression(branch, &mut private.clone(), returns, depth + 1) { return false; }
                let right = self.result_code.take();
                self.result_code = self.join_codes(left, right);
                true
            }
            K::Match { value, arms } => {
                if (self.finite && !self.copy_type(&value.ty))
                    || !self.expression(value, private, returns, depth + 1) { return false; }
                let mut codes = Some(vec![]);
                for arm in arms {
                    if !stable_pattern(&arm.pattern)
                        || !arm.guard.as_ref().is_none_or(|guard| self.copy_type(&guard.ty)
                            && self.expression(guard, &mut private.clone(), returns, depth + 1))
                        || !self.expression(&arm.body, &mut private.clone(), returns, depth + 1) { return false; }
                    let result = self.result_code.take();
                    codes = self.join_codes(codes, result);
                }
                self.result_code = codes;
                true
            }
            K::Call { callee, arguments, .. } => {
                if self.finite && !self.copy_type(&value.ty) { return false; }
                let candidates = if matches!(callee.kind, K::TraitMethod { .. }) {
                    vec![callee.as_ref().clone()]
                } else {
                    if !self.expression(callee, private, returns, depth + 1) { return false; }
                    let Some(candidates) = self.result_code.take() else { return false; };
                    candidates
                };
                if candidates.is_empty() { return false; }
                let mut inputs = BTreeMap::new();
                // Snapshot identities at each argument's preparation point.
                for arg in arguments {
                    if (self.finite && !self.copy_type(&arg.value.ty))
                        || !self.expression(&arg.value, private, returns, depth + 1) { return false; }
                    if let Some(codes) = self.result_code.take() {
                        let Some(keys) = codes.iter().map(|code| self.code_key(code)).collect::<Option<Vec<_>>>() else { return false; };
                        inputs.insert(arg.parameter, keys);
                    }
                }
                let mut returned = Some(vec![]);
                for candidate in candidates {
                    if !self.call_candidate(&candidate, &inputs, value, depth + 1) { return false; }
                    let result = self.result_code.take();
                    returned = self.join_codes(returned, result);
                }
                self.result_code = returned;
                true
            }
            K::Field { value: inner, .. } | K::AnonymousField { value: inner, .. } if self.finite => {
                // Observing a Copy field does not copy the owning aggregate.
                // A direct place path performs no supplier calls or temporary
                // resource construction; availability remains a move obligation.
                self.copy_type(&value.ty) && (self.stable_place_read(value)
                    || self.copy_type(&inner.ty) && self.expression(inner,private,returns,depth+1))
            }
            K::Field { value: inner, .. } | K::AnonymousField { value: inner, .. } if !self.finite =>
                self.expression(inner, private, returns, depth + 1),
            K::Record { fields, .. } | K::Variant { fields, .. } if !self.finite =>
                fields.iter().all(|field| self.expression(field, private, returns, depth + 1)),
            K::Discard(inner) => (!self.finite || self.copy_type(&inner.ty) || is_never(&inner.ty))
                && self.expression(inner, private, returns, depth + 1),
            K::Not(inner) | K::FloatNegate(inner)
            | K::BitConvert(inner) | K::Snapshot(inner) | K::TakeBits(inner) | K::SymbolName(inner) | K::EnumName(inner) | K::BitSlice { value: inner, .. }
            | K::AnonymousField { value: inner, .. } => {
                self.copy_type(&value.ty) && self.copy_type(&inner.ty)
                    && self.expression(inner, private, returns, depth + 1)
            }
            K::And(a,b) | K::Or(a,b) | K::ScalarLess(a,b) | K::ScalarLessEqual(a,b)
            | K::ScalarGreater(a,b) | K::ScalarGreaterEqual(a,b) | K::ScalarEqual(a,b)
            | K::BitEqual(a,b) | K::BitConcat(a,b) | K::FloatBinary { left: a, right: b, .. } => {
                self.copy_type(&value.ty) && self.expression(a, private, returns, depth + 1)
                    && self.expression(b, private, returns, depth + 1)
            }
            K::IntegerBinary { left, right, .. } => {
                n::machine_integer_type(value).is_some()
                    && self.expression(left, private, returns, depth + 1)
                    && self.expression(right, private, returns, depth + 1)
            }
            K::IntegerNegate(inner) => n::machine_integer_type(value).is_some()
                && self.expression(inner, private, returns, depth + 1),
            K::Tuple(fields) | K::AnonymousRecord { fields } => {
                (!self.finite || self.copy_type(&value.ty)) && fields.iter().all(|field|
                    self.expression(field, private, returns, depth + 1))
            }
            // Keep the existing constructor, pattern and code-value checks.
            // In particular, no fallback invokes a closure or invents Copy for
            // nominal values with potentially observable cleanup.
            _ => stable_default(value),
        }
    }

    /// Value transfer is not a state effect. Require the existing affine pass
    /// to account for every path before admitting non-Copy values: leftovers,
    /// replacement, failed preparation and partial construction may run Drop.
    fn transfer_cleanup_is_pure(&mut self, function: &n::Function, inputs: &Inputs) -> bool {
        use crate::verification::moves::{self, Delivery};
        let callable_parameters = inputs.keys().filter_map(|slot| {
            let parameter = function.parameters.get(*slot)?;
            let mut definitions = self.callables.get(parameter)?.clone();
            if definitions.is_empty() { return None; }
            // Resource evidence uses a closure representative for mixed sets.
            let closure = definitions.iter().position(|d| matches!(d.kind, K::Closure { .. }));
            let definition = definitions.remove(closure.unwrap_or(0));
            let (alternatives, choices) = if closure.is_some() {
                (vec![], definitions.into_iter().map(|definition| moves::CallableEvidence {
                    definition, alternatives: vec![], choices: vec![], captures: Default::default(), enum_captures: Default::default(),
                }).collect())
            } else { (definitions, vec![]) };
            Some(moves::CallableParameter {
                parameter: *slot, definition, fields: vec![], alternatives, choices,
                captures: Default::default(), enum_captures: Default::default(),
            })
        }).collect::<Vec<_>>();
        let mut copy_types = HashSet::new();
        if self.protocols.drop.is_none() {
            let report = moves::analyze_in_package_with_inputs(self.package, function, |ty|
                Ok(if self.copy_type(ty) { Delivery::Copy } else { Delivery::Move }), self.fuel, None, &[], &callable_parameters);
            let Ok(report) = report else { return false; };
            self.fuel = self.fuel.saturating_sub(report.entry_steps_used);
            return self.cleanup_report_is_pure(&report, &copy_types, None, 0);
        }
        let mut fuel = self.fuel;
        let context = crate::ptc::Context::from_package(self.package);
        let mut index = crate::implementation::Index::default();
        let budget = crate::ptc::Budget { steps: fuel, ..Default::default() };
        for implementation in &self.package.implementations {
            if implementation.interface.is_none() { continue; }
            if index.insert(crate::implementation::ImplementationHead {
                declaration: implementation.declaration,
                interface: implementation.interface,
                interface_arguments: implementation.interface_arguments.clone(),
                subject: implementation.subject.clone(),
                type_parameters: implementation.type_parameters.clone(),
                requirements: implementation.requirements.clone(),
            }, budget).is_err() { return false; }
        }
        let Ok(context) = context else { return false; };
        let world = crate::capabilities::World {
            index: &index, context: &context, traits: &self.package.traits,
            implementations: &self.package.implementations, assumptions: &function.capabilities,
        };
        let report = moves::analyze_in_package_with_inputs(self.package, function, |ty| {
            if self.copy_type(ty) || copy_types.contains(ty) { return Ok(Delivery::Copy); }
            let (Some(copy), Some(drop)) = (self.protocols.copy, self.protocols.drop) else {
                return Ok(Delivery::Move);
            };
            use crate::verification::copy::{self, Classification};
            match copy::classify(&world, copy, ty, budget)? {
                Classification::Intrinsic => { copy_types.insert(ty.clone()); Ok(Delivery::Copy) }
                Classification::Affine => Ok(Delivery::Move),
                Classification::Pending(reason) => Err(reason),
                _ => {
                    copy::acquire(self.package, &world, copy, drop, ty, budget, &mut fuel)?;
                    copy_types.insert(ty.clone());
                    Ok(Delivery::Copy)
                }
            }
        }, self.fuel, None, &[], &callable_parameters);
        self.fuel = fuel;
        let Ok(report) = report else { return false; };
        self.fuel = self.fuel.saturating_sub(report.entry_steps_used);
        self.cleanup_report_is_pure(&report, &copy_types, Some(&world), 0)
    }

    fn stable_place_read(&mut self,mut value:&n::Expr)->bool {
        loop {
            if self.fuel==0 {return false;}
            self.fuel-=1;
            match &value.kind {
                K::Local(_)=>return true,
                K::Field {value:inner,..} | K::AnonymousField {value:inner,..}=>value=inner,
                _=>return false,
            }
        }
    }

    fn trivial_internal_packet(&mut self,ty:&Term,depth:usize)->bool {
        if depth>=128 || self.fuel==0 {return false;}
        self.fuel-=1;
        let Term::Nominal {declaration,arguments}=ty else {return false};
        if !arguments.is_empty() || !self.package.internal_return_packets.contains(declaration) {return false;}
        let Some(packet)=self.package.enums.iter().find(|item|item.declaration==*declaration) else {return false};
        if !packet.type_parameters.is_empty() {return false;}
        // Only retirement is certified. Ownership delivery still consults the
        // ordinary Copy protocol, and nontrivial payloads still require Drop.
        packet.variants.iter().flat_map(|variant|&variant.fields)
            .all(|field|self.copy_type(field) || self.trivial_internal_packet(field,depth+1))
    }

    fn cleanup_report_is_pure(&mut self, report: &crate::verification::moves::Report, copy_types: &HashSet<Term>, world: Option<&crate::capabilities::World<'_>>, depth: usize) -> bool {
        use crate::verification::moves::CleanupAction;
        if depth >= 128 || !report.deferred_calls.is_empty() { return false; }
        for site in &report.cleanup_sites {
            let mut actions: Vec<_> = site.actions.iter().collect();
            while let Some(action) = actions.pop() {
                if self.fuel == 0 { return false; }
                self.fuel -= 1;
                // Scalars and acquired Copy representations have no user cleanup.
                // Other responsibilities need a separate Drop behavior contract.
                let ty = match action {
                    CleanupAction::Alternatives(cases) => {
                        if cases.len() > self.fuel { return false; }
                        self.fuel -= cases.len();
                        actions.extend(cases);
                        continue;
                    }
                    CleanupAction::Owned { ty, .. } => ty,
                    CleanupAction::Temporary { temporary, .. } => &temporary.ty,
                    CleanupAction::Prepared { value, .. } => &value.ty,
                    CleanupAction::Planned { .. } => return false,
                };
                if let Term::Nominal {declaration,..}=ty {
                    let remaining=match action {
                        CleanupAction::Owned {remaining,..} | CleanupAction::Temporary {remaining,..}=>Some(remaining),
                        _=>None,
                    };
                    if self.package.internal_return_packets.contains(declaration)
                        && remaining.is_some_and(|tree|tree.children.as_ref().is_some_and(Vec::is_empty)) {
                        // The payload has already moved out. Only the known
                        // compiler envelope remains; no T destructor is skipped.
                        continue;
                    }
                }
                if site.reason==crate::verification::moves::CleanupReason::Replacement {
                    if let (CleanupAction::Owned {..},Term::Nominal {declaration,..})=(action,ty) {
                        if self.package.internal_return_packets.contains(declaration)
                            && self.package.internal_return_empty_replacements.get(&(site.source.start,site.source.end))==Some(declaration) {
                            continue;
                        }
                    }
                }
                if !self.copy_type(ty) && !copy_types.contains(ty) && !self.trivial_internal_packet(ty,0) {
                    let (Some(world), Some(protocol)) = (world, self.protocols.drop) else { return false; };
                    let budget = crate::ptc::Budget { steps: self.fuel, ..Default::default() };
                    let Ok(cleanup) = crate::verification::drop::acquire_cleanup(
                        self.package, world, protocol, ty, budget, &mut self.fuel) else { return false; };
                    // Every potentially invoked destructor needs body evidence;
                    // this does not weaken Copy's prohibition on duplicating
                    // custom Drop responsibility or the separate access checks.
                    let mut methods = Vec::new();
                    if cleanup.visit_components(&mut self.fuel, &mut |_, resolution, fuel| {
                        use crate::verification::drop::Resolution;
                        match resolution {
                            Resolution::NoCustomDrop(_) => Ok(()),
                            Resolution::Selected(selected) => {
                                for argument in &selected.method.arguments {
                                    crate::associated::charge_projection_copy(argument, fuel)?;
                                }
                                methods.push(selected.method.clone());
                                Ok(())
                            }
                            _ => Err("cleanup selection is unresolved".into()),
                        }
                    }).is_err() { return false; }
                    if !methods.iter().all(|method| self.function(method.body, &method.arguments, depth + 1)) {
                        return false;
                    }
                }
            }
        }
        report.handlers.iter().all(|handler| self.cleanup_report_is_pure(&handler.invocation, copy_types, world, depth + 1))
            && report.defaults.iter().all(|default| self.cleanup_report_is_pure(&default.evaluation, copy_types, world, depth + 1))
            && report.closures.iter().filter(|closure| closure.invoked)
                .all(|closure| self.cleanup_report_is_pure(&closure.invocation, copy_types, world, depth + 1))
    }

    /// Flow-insensitive loop closure covers every program point (including
    /// early exits), not just the final state of one syntactic body traversal.
    fn loop_code_closure(&mut self, body: &n::Expr) -> bool {
        let mut equations = Vec::new();
        let mut locals = BTreeSet::new();
        if nir::visit::expression(body, &mut self.fuel, |node, _| {
            match &node.kind {
                K::Let { binding, value } if matches!(value.ty, Term::Function { .. } | Term::Closure { .. }) => {
                    locals.insert(*binding);
                    equations.push((*binding, value.as_ref()));
                }
                K::Assign { place, value } if matches!(value.ty, Term::Function { .. } | Term::Closure { .. }) => {
                    equations.push((place.root, value.as_ref()));
                }
                _ => {}
            }
            Ok(())
        }).is_err() { return false; }
        let mut facts: BTreeMap<_, _> = self.callables.iter().map(|(id, codes)| (*id, Some(codes.clone()))).collect();
        for local in locals { facts.entry(local).or_insert_with(|| Some(vec![])); }
        for (target, _) in &equations { facts.entry(*target).or_insert(None); }
        loop {
            if self.fuel < facts.len().saturating_add(1) { self.fuel = 0; return false; }
            self.fuel -= facts.len() + 1;
            let mut changed = false;
            for (target, value) in &equations {
                let incoming = self.loop_code_value(value, &facts, 0);
                let previous = facts.get(target).cloned().flatten();
                let next = self.join_codes(previous.clone(), incoming);
                if next != previous { facts.insert(*target, next); changed = true; }
                if self.fuel == 0 { return false; }
            }
            if !changed { break; }
        }
        // Only entry bindings survive the loop's lexical scope. Local equations
        // were needed to close aliases; their execution still initializes them.
        self.callables.retain(|binding, codes| match facts.get(binding) {
            Some(Some(incoming)) if !incoming.is_empty() => { *codes = incoming.clone(); true }
            _ => false,
        });
        true
    }

    fn loop_code_value(&mut self, value: &n::Expr, facts: &BTreeMap<n::LocalId, Option<Codes>>, depth: usize) -> Option<Codes> {
        if self.fuel == 0 || depth >= 128 { return None; }
        self.fuel -= 1;
        match &value.kind {
            K::Function(_) | K::FunctionInstance { .. } => Some(vec![value.clone()]),
            K::Closure { captures, .. } if self.supported_captures(captures) => Some(vec![value.clone()]),
            K::Local(binding) => facts.get(binding).cloned().flatten(),
            K::If { then_branch, else_branch: Some(other), .. } => {
                let left = self.loop_code_value(then_branch, facts, depth + 1);
                let right = self.loop_code_value(other, facts, depth + 1);
                self.join_codes(left, right)
            }
            K::Match { arms, .. } => {
                let mut codes = Some(vec![]);
                for arm in arms {
                    let incoming = self.loop_code_value(&arm.body, facts, depth + 1);
                    codes = self.join_codes(codes, incoming);
                }
                codes
            }
            K::Call { callee, arguments, .. } => {
                // This phase acquires identities only. The subsequent behavior
                // walk checks the factory, argument effects and every result.
                let callees = self.loop_code_value(callee, facts, depth + 1)?;
                if callees.is_empty() { return None; }
                let mut inputs = Inputs::new();
                for argument in arguments {
                    if !matches!(argument.value.ty, Term::Function { .. } | Term::Closure { .. }) { continue; }
                    if let Some(codes) = self.loop_code_value(&argument.value, facts, depth + 1) {
                        // An unresolved recursive seed is not an executable
                        // code premise. Keep it conservative until established.
                        if codes.is_empty() { return None; }
                        let keys = codes.iter().map(|code| self.code_key(code)).collect::<Option<Vec<_>>>()?;
                        inputs.insert(argument.parameter, keys);
                    }
                }
                let mut outputs = Some(vec![]);
                for callee in callees {
                    let (id, types) = match &callee.kind {
                        K::Function(id) => (*id, vec![]),
                        K::FunctionInstance { declaration, arguments } => (*declaration, arguments.clone()),
                        _ => return None,
                    };
                    let returned = self.returned_code(id, &types, &inputs, value);
                    outputs = self.join_codes(outputs, returned);
                }
                outputs
            }
            K::Block { result: Some(result), .. } | K::CallPreparation { result: Some(result), .. } =>
                self.loop_code_value(result, facts, depth + 1),
            _ => None,
        }
    }

    fn join_codes(&mut self, left: Option<Codes>, right: Option<Codes>) -> Option<Codes> {
        let (Some(mut left), Some(right)) = (left, right) else { return None; };
        for code in right {
            let Some(remaining) = self.fuel.checked_sub(left.len().saturating_add(1)) else {
                self.fuel = 0;
                return None;
            };
            self.fuel = remaining;
            if !left.iter().any(|old| old.ty == code.ty && (old.kind == code.kind || match (&old.kind, &code.kind) {
                (K::Function(a), K::FunctionInstance { declaration: b, arguments })
                | (K::FunctionInstance { declaration: b, arguments }, K::Function(a)) => a == b && arguments.is_empty(),
                _ => false,
            })) { left.push(code); }
        }
        Some(left)
    }

    fn call_candidate(&mut self, callee: &n::Expr, inputs: &Inputs, result: &n::Expr, depth: usize) -> bool {
        self.result_code = None;
        let (id, types) = match &callee.kind {
            K::Function(id) => (*id, vec![]),
            K::FunctionInstance { declaration, arguments } => (*declaration, arguments.clone()),
            K::Closure { captures, .. } if self.supported_captures(captures) => {
                let valid = self.closure(callee, inputs, depth + 1);
                self.result_code = None;
                return valid;
            }
            K::TraitMethod { interface, member, .. } => {
                let promised = self.package.traits.iter().find(|t| t.declaration == *interface)
                    .and_then(|t| t.methods.iter().find(|m| m.declaration == *member))
                    .is_some_and(|m| m.pure && !m.signature.is_comptime_callable());
                if promised && !self.finite { return true; }
                let Ok(method) = crate::materialization::resolve_method_instance(self.package, callee, &mut self.fuel) else { return false; };
                (method.body, method.arguments)
            }
            _ => return false,
        };
        let valid = self.function_with_inputs(id, &types, inputs, depth + 1);
        self.result_code = None;
        if valid && matches!(result.ty, Term::Function { .. } | Term::Closure { .. }) {
            self.result_code = self.returned_code(id, &types, inputs, result);
        }
        valid
    }

    /// Recover identities, never a behavior certificate. Every invoking caller
    /// must separately check the factory and its argument computations.
    fn returned_code(&mut self, id: DeclarationId, types: &[Term], inputs: &Inputs, result: &n::Expr) -> Option<Codes> {
        use crate::verification::memory::{self, CallableInput, CodeDefinition, CodeResult, PreparedCode};
        let mut definitions = BTreeMap::new();
        for (slot, codes) in inputs {
            let definitions_for_slot = codes.iter().map(|code| self.code_value(code, &result.ty, result.source)).collect::<Option<Vec<_>>>()?;
            definitions.insert(CallableInput { parameter: *slot, fields: vec![] }, definitions_for_slot);
        }
        let prepared = definitions.iter().map(|(input, definitions)| {
            let values = definitions.iter().map(|definition| match &definition.kind {
                K::FunctionInstance { declaration, arguments } => CodeDefinition::Named(iir::InstanceKey { declaration: *declaration, arguments: arguments.clone() }),
                _ => CodeDefinition::Closure { definition, captures: Default::default(), prepared_captures: Default::default() },
            }).collect();
            (input.clone(), values)
        }).collect();
        let summary = memory::infer_with_code_inputs(self.package, id, types, &BTreeMap::new(), &prepared, &mut self.fuel).ok()?;
        let CodeResult::Known(candidates) = summary.code_fields.get(&vec![])? else { return None; };
        if candidates.is_empty() { return None; }
        candidates.iter().map(|candidate| match candidate {
            PreparedCode::Named(key) => Some(n::Expr {
                source: result.source, ty: result.ty.clone(),
                kind: K::FunctionInstance { declaration: key.declaration, arguments: key.arguments.clone() },
            }),
            PreparedCode::Closure { definition, captures, prepared_captures }
                if captures.is_empty() && prepared_captures.is_empty() => {
                    let K::Closure { captures, .. } = &definition.kind else { return None; };
                    self.supported_captures(captures).then(|| definition.as_ref().clone())
                }
            _ => None,
        }).collect()
    }

    fn supported_captures(&self, captures: &[n::Capture]) -> bool {
        // P4 can transfer affine inputs into an environment. Invocation and
        // retirement are checked by the same move/cleanup reports as ordinary
        // owned parameters; capture types alone do not grant Copy or purity.
        captures.is_empty() || !self.finite
    }

    fn code_key(&mut self, code: &n::Expr) -> Option<CodeKey> {
        match &code.kind {
            K::Function(id) => Some(CodeKey::Named(*id, vec![])),
            K::FunctionInstance { declaration, arguments } => Some(CodeKey::Named(*declaration, arguments.clone())),
            K::Closure { captures, .. } if self.supported_captures(captures) => {
                let initial_fuel = self.fuel;
                let mut fuel = self.fuel as u64;
                let bounded = constants::charge_copy(code, &mut fuel, Span::default()).is_ok();
                self.fuel = fuel as usize;
                if !bounded { return None; }
                let comparison_cost = initial_fuel.saturating_sub(self.fuel).max(1);
                for (index, definition) in self.closures.iter().enumerate() {
                    if self.fuel < comparison_cost { self.fuel = 0; return None; }
                    self.fuel -= comparison_cost;
                    if definition == code { return Some(CodeKey::Closure(index)); }
                }
                let index = self.closures.len();
                self.closures.push(code.clone());
                Some(CodeKey::Closure(index))
            }
            _ => None,
        }
    }

    fn code_value(&self, key: &CodeKey, ty: &Term, source: n::SourceRange) -> Option<n::Expr> {
        match key {
            CodeKey::Named(declaration, arguments) => Some(n::Expr {
                source, ty: ty.clone(), kind: K::FunctionInstance { declaration: *declaration, arguments: arguments.clone() },
            }),
            CodeKey::Closure(index) => self.closures.get(*index).cloned(),
        }
    }

    fn closure(&mut self, definition: &n::Expr, inputs: &Inputs, depth: usize) -> bool {
        if depth >= 128 || self.fuel == 0 { return false; }
        self.fuel -= 1;
        let K::Closure { parameters: bindings, captures, body } = &definition.kind else { return false; };
        if definition.ty.is_comptime_callable() { return false; }
        let Term::Function { parameters, result } = definition.ty.callable_signature() else { return false; };
        if !self.supported_captures(captures) || (self.finite && !self.copy_type(result))
            || parameters.iter().any(|p| p.sort != Sort::Runtime || (self.finite && !self.copy_type(&p.ty))) {
            return false;
        }
        let saved = std::mem::take(&mut self.callables);
        for (slot, key) in inputs {
            let (Some(binding), Some(parameter)) = (bindings.get(*slot), parameters.get(*slot)) else {
                self.callables = saved;
                return false;
            };
            let Some(code) = key.iter().map(|key| self.code_value(key, &parameter.ty, definition.source)).collect::<Option<Vec<_>>>() else {
                self.callables = saved;
                return false;
            };
            self.callables.insert(*binding, code);
        }
        // These are value environments: constructors create owned captures,
        // and formal closure arguments are delivered by value. The enclosing
        // move report must still validate delivery, aliases and every cleanup.
        // This permits updating captured scalar slots, not their pointees.
        let mut private = bindings.iter().copied().chain(captures.iter().map(|capture| capture.binding)).collect();
        let valid = self.expression(body, &mut private, true, depth + 1);
        self.callables = saved;
        valid
    }

    fn function(&mut self, id: DeclarationId, types: &[Term], depth: usize) -> bool {
        self.function_with_inputs(id, types, &BTreeMap::new(), depth)
    }

    fn function_with_inputs(&mut self, id: DeclarationId, types: &[Term], inputs: &Inputs, depth: usize) -> bool {
        if depth >= 128 || self.fuel == 0 { return false; }
        self.fuel -= 1;
        for ty in types {
            if crate::associated::charge_projection_copy(ty, &mut self.fuel).is_err() { return false; }
        }
        for code in inputs.values().flatten() {
            let CodeKey::Named(_, arguments) = code else { continue; };
            if self.fuel == 0 { return false; }
            self.fuel -= 1;
            for argument in arguments {
                if crate::associated::charge_projection_copy(argument, &mut self.fuel).is_err() { return false; }
            }
        }
        let key = (id, types.to_vec(), inputs.clone());
        // For P4, a backedge assumes only the same behavior obligation, not
        // termination. Every node in this cycle still has its body checked.
        // Failed roots abort validation, so provisional cycle facts never
        // escape into a successfully validated package.
        if self.active.contains(&key) { return !self.finite; }
        if self.active.iter().any(|(active, _, _)| *active == id) { return false; }
        if let Some(valid) = self.proven.get(&key) { return *valid; }
        let Some(function) = self.functions.get(&id).copied() else { return false; };
        if function.phase != n::FunctionPhase::Runtime || function.type_parameters.len() != types.len() {
            return false;
        }
        // Charge owned body/type copies before specializing. A long or
        // polymorphically growing call graph cannot bypass the analysis budget.
        let mut fuel = self.fuel as u64;
        let bounded = std::iter::once(&function.body)
            .chain(function.preconditions.iter()).chain(function.postconditions.iter())
            .chain(function.defaults.iter().map(|(_, value)| value))
            .all(|value| constants::charge_copy(value, &mut fuel, Span::default()).is_ok());
        self.fuel = fuel as usize;
        if !bounded || crate::associated::charge_projection_copy(&function.signature, &mut self.fuel).is_err() {
            return false;
        }
        // Instantiation substitutes types only. The source function and its
        // opaque type heads remain untouched, and runtime inputs are not read.
        let Ok(function) = function.instantiate_types(&function.type_parameters, types) else { return false; };
        let Term::Function { parameters, result } = &function.signature else { return false; };
        if (self.finite && !self.copy_type(result)) || parameters.iter().any(|parameter|
            parameter.sort != Sort::Runtime || (self.finite && !self.copy_type(&parameter.ty))) {
            return false;
        }
        let saved_callables = std::mem::take(&mut self.callables);
        for (slot, code) in inputs {
            let (Some(binding), Some(parameter)) = (function.parameters.get(*slot), parameters.get(*slot)) else {
                self.callables = saved_callables;
                return false;
            };
            let Some(definition) = code.iter().map(|key| self.code_value(key, &parameter.ty, function.body.source)).collect::<Option<Vec<_>>>() else {
                self.callables = saved_callables;
                return false;
            };
            self.callables.insert(*binding, definition);
        }
        let entry_callables = self.callables.clone();
        self.active.insert(key.clone());
        let mut private = function.parameters.iter().copied().collect();
        let valid = function.preconditions.iter().chain(&function.postconditions)
            .all(|value| self.copy_type(&value.ty) && self.expression(value, &mut private, false, depth + 1))
            && self.expression(&function.body, &mut private, true, depth + 1)
            && (self.finite || {
                self.callables = entry_callables;
                self.transfer_cleanup_is_pure(&function, inputs)
            });
        self.callables = saved_callables;
        self.active.remove(&key);
        self.proven.insert(key, valid);
        valid
    }
}
