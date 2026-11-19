//! Lexical value dependencies of a checked expression. Binding scopes here
//! follow interpretation; type-level bound slots are handled by substitution.
use crate::program::{Expr, ExprKind as K, LocalId, NormalFormArgument as A, Pattern as P};
use std::collections::{BTreeMap, BTreeSet};

mod lexical;
use lexical::{Alternatives, LexicalScope};

/// Resource limits must not be mistaken for malformed lexical scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Budget(&'static str),
    Depth(&'static str),
    Invalid(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Budget(message) | Self::Depth(message) | Self::Invalid(message) => {
                f.write_str(message)
            }
        }
    }
}
impl std::error::Error for Error {}
impl From<crate::visit::Error> for Error {
    fn from(error: crate::visit::Error) -> Self {
        match error {
            crate::visit::Error::Budget(message) => Self::Budget(message),
            crate::visit::Error::Invalid(message) => Self::Invalid(message),
        }
    }
}
// Text-only diagnostic consumers may discard the classification at their boundary.
impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.to_string()
    }
}

type Dependencies = BTreeMap<LocalId, Scope>;
type Scope = BTreeSet<LocalId>;
fn charge(fuel: &mut usize, amount: usize) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(amount)
        .ok_or(Error::Budget("capture analysis budget exhausted"))?;
    Ok(())
}
/// Expand one symbolic read in its use-site scope. Another use may be
/// shadowed by a different pattern, default, or closure binder.
fn read(
    mut id: LocalId,
    scope: &LexicalScope,
    free: &mut Scope,
    dependencies: &Dependencies,
    fuel: &mut usize,
) -> Result<(), Error> {
    enum Visit {
        Active,
        Complete,
    }
    let mut visits = BTreeMap::new();
    let mut path = Vec::new();
    loop {
        charge(fuel, 1)?;
        if !scope.contains(&id) {
            if let Some(inputs) = dependencies.get(&id) {
                match visits.entry(id) {
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(Visit::Active);
                        path.push((id, inputs.iter()));
                    }
                    std::collections::btree_map::Entry::Occupied(slot) => {
                        if matches!(slot.get(), Visit::Active) {
                            return Err(Error::Invalid("cyclic symbolic capture dependency"));
                        }
                    }
                }
            } else {
                free.insert(id);
            }
        }
        // Retain a cursor per active node. A wide dependency list need not
        // be copied or pre-enqueued before its first edge can be charged.
        loop {
            let Some((parent, edges)) = path.last_mut() else {
                return Ok(());
            };
            if let Some(next) = edges.next() {
                id = *next;
                break;
            }
            visits.insert(*parent, Visit::Complete);
            path.pop();
        }
    }
}
pub fn collect(expr: &Expr, fuel: &mut usize) -> Result<Scope, Error> {
    collect_with_dependencies(expr, &Dependencies::new(), fuel)
}
/// Bindings of the operand schedule after the callee has been selected.
pub struct InputPlanBindings {
    pub imports: BTreeSet<LocalId>,
    pub locals: Vec<LocalId>,
}

pub fn input_plan_bindings(expr: &Expr, fuel: &mut usize) -> Result<InputPlanBindings, Error> {
    let mut free = Scope::new();
    let locals = scan_inputs(
        expr,
        &mut LexicalScope::default(),
        &mut free,
        fuel,
        0,
        &Dependencies::new(),
    )?;
    Ok(InputPlanBindings {
        imports: free,
        locals,
    })
}

fn scan_inputs(
    node: &Expr,
    scope: &mut LexicalScope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<Vec<LocalId>, Error> {
    let (arguments, schedule) = node
        .kind
        .declaration_inputs()
        .ok_or(Error::Invalid("expected declaration input plan"))?;
    charge(fuel, arguments.len())?;
    let schedule = schedule
        .check(arguments.len(), |n| charge(fuel, n))?
        .ok_or(Error::Invalid("invalid declaration input plan"))?;
    let mark = scope.mark();
    for step in schedule.steps() {
        match step {
            crate::program::InputStep::Prepare(expr) => {
                statement(expr, scope, free, fuel, depth + 1, dependencies)?;
            }
            crate::program::InputStep::Supply(slot) => {
                normal(&arguments[slot], scope, free, fuel, depth + 1, dependencies)?;
                scope.extend(schedule.binding(slot));
            }
        }
    }
    if let K::CheckTypeAlias {
        body: Some(body), ..
    } = &node.kind
    {
        scan(body, scope, free, fuel, depth + 1, dependencies)?;
    }
    scope.restore(mark);
    Ok(schedule.locals)
}

/// Runtime dependencies of pattern observations, excluding lexical binders.
pub fn collect_pattern(pattern: &P, fuel: &mut usize) -> Result<Scope, Error> {
    collect_pattern_with_dependencies(pattern, &Dependencies::new(), fuel)
}

/// Symbolic reads in a pattern see only the bindings introduced at their
/// position. The resulting free set excludes the pattern's success outputs.
pub fn collect_pattern_with_dependencies(
    pattern: &P,
    dependencies: &Dependencies,
    fuel: &mut usize,
) -> Result<Scope, Error> {
    let mut free = Scope::new();
    pattern_scan(
        pattern,
        &mut LexicalScope::default(),
        &mut free,
        fuel,
        0,
        dependencies,
    )?;
    Ok(free)
}
/// As `collect`, with symbolic reads expanded in their lexical scope.
pub fn collect_with_dependencies(
    expr: &Expr,
    dependencies: &Dependencies,
    fuel: &mut usize,
) -> Result<Scope, Error> {
    let mut free = Scope::new();
    scan(
        expr,
        &mut LexicalScope::default(),
        &mut free,
        fuel,
        0,
        dependencies,
    )?;
    Ok(free)
}
/// Lexical dependencies of one handler/match arm, excluding its own pattern binders.
pub fn collect_arm(arm: &crate::program::MatchArm, fuel: &mut usize) -> Result<Scope, Error> {
    collect_arm_with_dependencies(arm, &Dependencies::new(), fuel)
}
/// Expand symbolic code dependencies at each lexical use, so a guard's own
/// pattern bindings are excluded while outer runtime dependencies remain free.
pub fn collect_arm_with_dependencies(
    arm: &crate::program::MatchArm,
    dependencies: &Dependencies,
    fuel: &mut usize,
) -> Result<Scope, Error> {
    let mut free = Scope::new();
    arm_scan(
        arm,
        &mut LexicalScope::default(),
        &mut free,
        fuel,
        0,
        dependencies,
    )?;
    Ok(free)
}
fn scan(
    expr: &Expr,
    scope: &mut LexicalScope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<(), Error> {
    if depth >= 128 {
        return Err(Error::Depth("capture analysis depth exhausted"));
    }
    crate::visit::expression_pruned_checked(expr, fuel, |node, fuel| {
        match &node.kind {
            K::Local(id) => read(*id, scope, free, dependencies, fuel)?,
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
                read(place.root, scope, free, dependencies, fuel)?
            }
            K::ComptimeFunctionInstance { inputs, .. } => {
                let mark = scope.mark();
                for value in &inputs.statements {
                    statement(value, scope, free, fuel, depth + 1, dependencies)?;
                }
                for value in &inputs.arguments {
                    scan(value, scope, free, fuel, depth + 1, dependencies)?;
                }
                scope.restore(mark);
                return Ok(false);
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mark = scope.mark();
                for value in statements {
                    statement(value, scope, free, fuel, depth + 1, dependencies)?;
                }
                if let Some(value) = result {
                    scan(value, scope, free, fuel, depth + 1, dependencies)?;
                }
                scope.restore(mark);
                return Ok(false);
            }
            K::LetPattern { .. } => {
                let mark = scope.mark();
                statement(node, scope, free, fuel, depth + 1, dependencies)?;
                scope.restore(mark);
                return Ok(false);
            }
            K::Match { value, arms } => {
                scan(value, scope, free, fuel, depth + 1, dependencies)?;
                for arm in arms {
                    arm_scan(arm, scope, free, fuel, depth + 1, dependencies)?;
                }
                return Ok(false);
            }
            K::Closure {
                captures,
                parameters,
                body,
            } => {
                charge(fuel, captures.len().saturating_add(parameters.len()))?;
                let mut inner = LexicalScope::with_parameters(parameters);
                for capture in captures {
                    if let Some(value) = &capture.value {
                        scan(value, scope, free, fuel, depth + 1, dependencies)?;
                    } else {
                        read(capture.binding, scope, free, dependencies, fuel)?;
                    }
                    inner.insert(capture.binding);
                }
                let mut missing = Scope::new();
                scan(
                    body,
                    &mut inner,
                    &mut missing,
                    fuel,
                    depth + 1,
                    dependencies,
                )?;
                if !missing.is_empty() {
                    return Err(Error::Invalid(
                        "closure body references an undeclared capture",
                    ));
                }
                return Ok(false);
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                scan(callee, scope, free, fuel, depth + 1, dependencies)?;
                let mut defaults = LexicalScope::default();
                for argument in arguments {
                    if argument.is_default {
                        let mut missing = Scope::new();
                        scan(
                            &argument.value,
                            &mut defaults,
                            &mut missing,
                            fuel,
                            depth + 1,
                            dependencies,
                        )?;
                        if !missing.is_empty() {
                            return Err(Error::Invalid(
                                "call default references an unavailable parameter",
                            ));
                        }
                    } else {
                        scan(&argument.value, scope, free, fuel, depth + 1, dependencies)?;
                    }
                    if let Some(binding) = parameter_bindings.get(argument.parameter) {
                        defaults.insert(*binding);
                    }
                }
                return Ok(false);
            }
            K::NominalType { .. }
            | K::CheckTypeAlias { .. }
            | K::NormalFormType { .. }
            | K::ApplyNormalForm { .. } => {
                if let K::ApplyNormalForm { callee, .. } = &node.kind {
                    scan(callee, scope, free, fuel, depth + 1, dependencies)?;
                }
                scan_inputs(node, scope, free, fuel, depth, dependencies)?;
                return Ok(false);
            }
            K::EffectHandle {
                computation,
                handlers,
                ..
            } => {
                scan(computation, scope, free, fuel, depth + 1, dependencies)?;
                for handler in handlers {
                    for arm in &handler.arms {
                        arm_scan(arm, scope, free, fuel, depth + 1, dependencies)?;
                    }
                }
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    })
}
fn statement(
    expr: &Expr,
    scope: &mut LexicalScope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<(), Error> {
    charge(fuel, 1)?;
    match &expr.kind {
        K::Let { binding, value } => {
            scan(value, scope, free, fuel, depth + 1, dependencies)?;
            scope.insert(*binding);
        }
        K::LetPattern {
            pattern,
            value,
            guard,
            otherwise,
        } => {
            scan(value, scope, free, fuel, depth + 1, dependencies)?;
            let mark = scope.mark();
            pattern_scan(pattern, scope, free, fuel, depth + 1, dependencies)?;
            if let Some(guard) = guard {
                scan(guard, scope, free, fuel, depth + 1, dependencies)?;
            }
            if let Some(otherwise) = otherwise {
                let success = scope.take_success(mark, fuel)?;
                scan(otherwise, scope, free, fuel, depth + 1, dependencies)?;
                scope.apply(success);
            }
        }
        _ => scan(expr, scope, free, fuel, depth + 1, dependencies)?,
    }
    Ok(())
}
fn arm_scan(
    arm: &crate::program::MatchArm,
    scope: &mut LexicalScope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<(), Error> {
    let mark = scope.mark();
    pattern_scan(&arm.pattern, scope, free, fuel, depth + 1, dependencies)?;
    if let Some(guard) = &arm.guard {
        scan(guard, scope, free, fuel, depth + 1, dependencies)?;
    }
    scan(&arm.body, scope, free, fuel, depth + 1, dependencies)?;
    scope.restore(mark);
    Ok(())
}
fn pattern_scan(
    pattern: &P,
    scope: &mut LexicalScope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<(), Error> {
    if depth >= 128 {
        return Err(Error::Depth("capture pattern depth exhausted"));
    }
    charge(fuel, 1)?;
    match pattern {
        P::CodeEvidence { outputs } => {
            charge(fuel, outputs.len())?;
            scope.extend(outputs.iter().map(|(id, _)| *id));
        }

        P::Observe { binding, inner } => {
            let inserted = scope.insert(*binding);
            pattern_scan(inner, scope, free, fuel, depth + 1, dependencies)?;
            if inserted {
                scope.remove(binding);
            }
        }

        P::Binding { binding, inner } => {
            pattern_scan(inner, scope, free, fuel, depth + 1, dependencies)?;
            scope.insert(*binding);
        }
        P::Then { first, value, next } => {
            pattern_scan(first, scope, free, fuel, depth + 1, dependencies)?;
            scan(value, scope, free, fuel, depth + 1, dependencies)?;
            pattern_scan(next, scope, free, fuel, depth + 1, dependencies)?;
        }
        P::Guard { pattern, condition } => {
            pattern_scan(pattern, scope, free, fuel, depth + 1, dependencies)?;
            scan(condition, scope, free, fuel, depth + 1, dependencies)?;
        }
        P::Remap { pattern, bindings } => {
            pattern_scan(pattern, scope, free, fuel, depth + 1, dependencies)?;
            charge(fuel, bindings.len())?;
            for (from, _) in bindings {
                read(*from, scope, free, dependencies, fuel)?;
            }
            let targets: Scope = bindings.iter().map(|(_, to)| *to).collect();
            for (from, _) in bindings {
                if !targets.contains(from) {
                    scope.remove(from);
                }
            }
            scope.extend(targets);
        }
        P::Or(patterns) => {
            let mark = scope.mark();
            let mut alternatives = Alternatives::default();
            for pattern in patterns {
                pattern_scan(pattern, scope, free, fuel, depth + 1, dependencies)?;
                alternatives.accept(scope, mark, fuel)?;
            }
            alternatives.finish(scope);
        }
        P::OptionalPresent(inner) | P::ErrorResult { inner, .. } => {
            pattern_scan(inner, scope, free, fuel, depth + 1, dependencies)?
        }
        P::NominalType { fields, .. }
        | P::Record { fields, .. }
        | P::AnonymousRecord { fields, .. }
        | P::Variant { fields, .. }
        | P::NormalForm {
            arguments: fields, ..
        }
        | P::Tuple(fields)
        | P::TypeTuple(fields) => {
            for field in fields {
                pattern_scan(field, scope, free, fuel, depth + 1, dependencies)?;
            }
        }
        P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
    }
    Ok(())
}
fn normal(
    argument: &A,
    scope: &mut LexicalScope,
    free: &mut Scope,
    fuel: &mut usize,
    depth: usize,
    dependencies: &Dependencies,
) -> Result<(), Error> {
    if depth >= 128 {
        return Err(Error::Depth("capture normal-input depth exhausted"));
    }
    charge(fuel, 1)?;
    match argument {
        A::Value(value) | A::ListSpread(value) => {
            scan(value, scope, free, fuel, depth + 1, dependencies)?
        }
        A::List { items, .. } | A::Tuple { items, .. } => {
            for item in items {
                normal(item, scope, free, fuel, depth + 1, dependencies)?;
            }
        }
        A::Block {
            statements, result, ..
        } => {
            let mark = scope.mark();
            for value in statements {
                statement(value, scope, free, fuel, depth + 1, dependencies)?;
            }
            if let Some(value) = result {
                normal(value, scope, free, fuel, depth + 1, dependencies)?;
            }
            scope.restore(mark);
        }
        A::If {
            condition,
            then_input,
            else_input,
            ..
        } => {
            scan(condition, scope, free, fuel, depth + 1, dependencies)?;
            normal(then_input, scope, free, fuel, depth + 1, dependencies)?;
            normal(else_input, scope, free, fuel, depth + 1, dependencies)?;
        }
        A::Match { value, arms, .. } => {
            scan(value, scope, free, fuel, depth + 1, dependencies)?;
            for arm in arms {
                let mark = scope.mark();
                pattern_scan(&arm.pattern, scope, free, fuel, depth + 1, dependencies)?;
                if let Some(guard) = &arm.guard {
                    scan(guard, scope, free, fuel, depth + 1, dependencies)?;
                }
                normal(&arm.body, scope, free, fuel, depth + 1, dependencies)?;
                scope.restore(mark);
            }
        }
        A::Integer { .. } | A::Unit { .. } | A::Null { .. } => {}
    }
    Ok(())
}
