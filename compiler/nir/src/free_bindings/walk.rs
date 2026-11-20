//! A lexical action can schedule expressions, patterns and contextual operands
//! on the same frontier. Only the active path can change the current scope.
use super::lexical::Edit;
use super::{
    Alternatives, Dependencies, Error, InputPlanBindings, LexicalScope, Scope, charge, read,
};
use crate::program::{CallArgument, Capture, InputStep, MatchArm, NormalMatchArm};
use crate::program::{Expr, ExprKind as K, LocalId, NormalFormArgument as A, Pattern as P};

pub fn input_plan_bindings(expr: &Expr, fuel: &mut usize) -> Result<InputPlanBindings, Error> {
    let mut walk = Walk::new();
    let locals = walk.schedule_inputs(expr, fuel)?;
    let imports = walk.finish(&Dependencies::new(), fuel)?;
    Ok(InputPlanBindings { imports, locals })
}

/// Symbolic reads in a pattern see only the bindings introduced at their
/// position. The resulting free set excludes the pattern's success outputs.
pub fn collect_pattern_with_dependencies(
    pattern: &P,
    dependencies: &Dependencies,
    fuel: &mut usize,
) -> Result<Scope, Error> {
    Walk::collect(Step::Pattern(pattern), dependencies, fuel)
}

/// As `collect`, with symbolic reads expanded in their lexical scope.
pub fn collect_with_dependencies(
    expr: &Expr,
    dependencies: &Dependencies,
    fuel: &mut usize,
) -> Result<Scope, Error> {
    Walk::collect(Step::Expr(expr), dependencies, fuel)
}

/// Expand symbolic code dependencies at each lexical use, so a guard's own
/// pattern bindings are excluded while outer runtime dependencies remain free.
pub fn collect_arm_with_dependencies(
    arm: &MatchArm,
    dependencies: &Dependencies,
    fuel: &mut usize,
) -> Result<Scope, Error> {
    Walk::collect(Step::Arm(arm), dependencies, fuel)
}

#[derive(Default)]
struct Environment {
    scope: LexicalScope,
    free: Scope,
}

// An ordinary call's defaults see its completed parameter prefix. While a
// default is being checked, this frame holds the caller's environment instead.
struct Call<'a> {
    arguments: &'a [CallArgument],
    bindings: &'a [LocalId],
    environment: Environment,
}

struct Branches<'a> {
    rest: &'a [P],
    common: Alternatives,
    mark: usize,
}

enum Step<'a> {
    Expr(&'a Expr),
    Statement(&'a Expr),
    Pattern(&'a P),
    Argument(&'a A),
    Arm(&'a MatchArm),
    InputArm(&'a NormalMatchArm),
    Inputs(&'a Expr),
    Restore(usize),
    Bind(LocalId),
    Remove(LocalId),
    Capture(&'a Capture),
    ClosureBody(&'a [LocalId], &'a [Capture], &'a Expr),
    LeaveClosure(Box<Environment>),
    Call(Box<Call<'a>>),
    AfterDefault(Box<Call<'a>>, Option<LocalId>),
    Otherwise(usize, &'a Expr),
    Commit(Vec<Edit>),
    Remap(&'a [(LocalId, LocalId)]),
    AfterAlternative(Box<Branches<'a>>),
}

impl<'a> From<crate::visit::Node<'a>> for Step<'a> {
    fn from(node: crate::visit::Node<'a>) -> Self {
        match node {
            crate::visit::Node::Expr(expr) => Self::Expr(expr),
            crate::visit::Node::Pattern(pattern) => Self::Pattern(pattern),
            crate::visit::Node::Argument(arg) => Self::Argument(arg),
        }
    }
}

struct Walk<'a> {
    tasks: Vec<Step<'a>>,
    current: Environment,
}

impl<'a> Walk<'a> {
    fn new() -> Self {
        Self {
            tasks: vec![],
            current: Environment::default(),
        }
    }

    fn collect(
        root: Step<'a>,
        dependencies: &Dependencies,
        fuel: &mut usize,
    ) -> Result<Scope, Error> {
        let mut walk = Self::new();
        walk.tasks.push(root);
        walk.finish(dependencies, fuel)
    }

    fn finish(mut self, dependencies: &Dependencies, fuel: &mut usize) -> Result<Scope, Error> {
        while let Some(step) = self.tasks.pop() {
            match step {
                Step::Expr(expr) => self.expression(expr, dependencies, fuel)?,
                Step::Pattern(pattern) => self.pattern(pattern, fuel)?,
                Step::Argument(value) => self.argument(value, fuel)?,
                Step::Statement(expr) => self.statement(expr, fuel)?,
                Step::Arm(arm) => self.arm(&arm.pattern, arm.guard.as_ref(), Step::Expr(&arm.body)),
                Step::InputArm(arm) => {
                    self.arm(&arm.pattern, arm.guard.as_ref(), Step::Argument(&arm.body))
                }
                Step::Inputs(expr) => {
                    self.schedule_inputs(expr, fuel)?;
                }
                Step::Restore(mark) => self.current.scope.restore(mark),
                Step::Bind(id) => {
                    self.current.scope.insert(id);
                }
                Step::Remove(id) => self.current.scope.remove(&id),
                Step::Capture(capture) => {
                    if let Some(value) = &capture.value {
                        self.tasks.push(Step::Expr(value));
                    } else {
                        self.read(capture.binding, dependencies, fuel)?;
                    }
                }
                Step::ClosureBody(parameters, captures, body) => {
                    let mut scope = LexicalScope::with_parameters(parameters);
                    scope.extend(captures.iter().map(|capture| capture.binding));
                    let caller = std::mem::replace(
                        &mut self.current,
                        Environment {
                            scope,
                            free: Scope::new(),
                        },
                    );
                    self.tasks.push(Step::LeaveClosure(Box::new(caller)));
                    self.tasks.push(Step::Expr(body));
                }
                Step::LeaveClosure(caller) => {
                    if !self.current.free.is_empty() {
                        return Err(Error::Invalid(
                            "closure body references an undeclared capture",
                        ));
                    }
                    self.current = *caller;
                }
                Step::Call(mut call) => {
                    if let Some((arg, rest)) = call.arguments.split_first() {
                        call.arguments = rest;
                        let binding = call.bindings.get(arg.parameter).copied();
                        if arg.is_default {
                            std::mem::swap(&mut self.current, &mut call.environment);
                            self.tasks.push(Step::AfterDefault(call, binding));
                        } else {
                            call.environment.scope.extend(binding);
                            self.tasks.push(Step::Call(call));
                        }
                        self.tasks.push(Step::Expr(&arg.value));
                    }
                }
                Step::AfterDefault(mut call, binding) => {
                    if !self.current.free.is_empty() {
                        return Err(Error::Invalid(
                            "call default references an unavailable parameter",
                        ));
                    }
                    std::mem::swap(&mut self.current, &mut call.environment);
                    call.environment.scope.extend(binding);
                    self.tasks.push(Step::Call(call));
                }
                Step::Otherwise(mark, expr) => {
                    let success = self.current.scope.take_success(mark, fuel)?;
                    self.tasks.push(Step::Commit(success));
                    self.tasks.push(Step::Expr(expr));
                }
                Step::Commit(edits) => self.current.scope.apply(edits),
                Step::Remap(bindings) => {
                    charge(fuel, bindings.len())?;
                    for (from, _) in bindings {
                        self.read(*from, dependencies, fuel)?;
                    }
                    let targets: Scope = bindings.iter().map(|(_, to)| *to).collect();
                    for (from, _) in bindings {
                        if !targets.contains(from) {
                            self.current.scope.remove(from);
                        }
                    }
                    self.current.scope.extend(targets);
                }
                Step::AfterAlternative(mut branches) => {
                    branches
                        .common
                        .accept(&mut self.current.scope, branches.mark, fuel)?;
                    if let Some((next, rest)) = branches.rest.split_first() {
                        branches.rest = rest;
                        self.tasks.push(Step::AfterAlternative(branches));
                        self.tasks.push(Step::Pattern(next));
                    } else {
                        branches.common.finish(&mut self.current.scope);
                    }
                }
            }
        }
        Ok(self.current.free)
    }

    fn read(
        &mut self,
        id: LocalId,
        dependencies: &Dependencies,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        read(
            id,
            &self.current.scope,
            &mut self.current.free,
            dependencies,
            fuel,
        )
    }

    fn scope(&mut self) {
        self.tasks.push(Step::Restore(self.current.scope.mark()));
    }

    // Charge before reserving space for siblings. Actual node visits and
    // binding operations still use the caller's remaining budget.
    fn sequence<T>(
        &mut self,
        items: &'a [T],
        step: impl Fn(&'a T) -> Step<'a>,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        charge(fuel, items.len())?;
        self.tasks.extend(items.iter().rev().map(step));
        Ok(())
    }

    fn arm(&mut self, pattern: &'a P, guard: Option<&'a Expr>, body: Step<'a>) {
        self.scope();
        self.tasks.push(body);
        self.tasks.extend(guard.map(Step::Expr));
        self.tasks.push(Step::Pattern(pattern));
    }

    fn schedule_inputs(&mut self, node: &'a Expr, fuel: &mut usize) -> Result<Vec<LocalId>, Error> {
        let (arguments, schedule) = node
            .kind
            .declaration_inputs()
            .ok_or(Error::Invalid("expected declaration input plan"))?;
        charge(fuel, arguments.len())?;
        let schedule = schedule
            .check(arguments.len(), |n| charge(fuel, n))?
            .ok_or(Error::Invalid("invalid declaration input plan"))?;
        self.scope();
        if let K::CheckTypeAlias {
            body: Some(body), ..
        } = &node.kind
        {
            self.tasks.push(Step::Expr(body));
        }
        let start = self.tasks.len();
        for step in schedule.steps() {
            match step {
                InputStep::Prepare(expr) => self.tasks.push(Step::Statement(expr)),
                InputStep::Supply(slot) => {
                    self.tasks.push(Step::Argument(&arguments[slot]));
                    self.tasks.extend(schedule.binding(slot).map(Step::Bind));
                }
            }
        }
        self.tasks[start..].reverse();
        Ok(schedule.locals)
    }

    fn expression(
        &mut self,
        expr: &'a Expr,
        dependencies: &Dependencies,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        charge(fuel, 1)?;
        match &expr.kind {
            K::Local(id) => self.read(*id, dependencies, fuel)?,
            K::ComptimeFunctionInstance { inputs, .. } => {
                self.scope();
                self.sequence(&inputs.arguments, Step::Expr, fuel)?;
                self.sequence(&inputs.statements, Step::Statement, fuel)?;
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                self.scope();
                self.tasks.extend(result.as_deref().map(Step::Expr));
                self.sequence(statements, Step::Statement, fuel)?;
            }
            K::LetPattern { .. } => {
                self.scope();
                self.tasks.push(Step::Statement(expr));
            }
            K::Match { value, arms } => {
                self.sequence(arms, Step::Arm, fuel)?;
                self.tasks.push(Step::Expr(value));
            }
            K::Closure {
                captures,
                parameters,
                body,
            } => {
                charge(fuel, parameters.len())?;
                self.tasks
                    .push(Step::ClosureBody(parameters, captures, body));
                self.sequence(captures, Step::Capture, fuel)?;
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
                env_arguments,
            } => {
                if !arguments.is_empty() {
                    self.tasks.push(Step::Call(Box::new(Call {
                        arguments,
                        bindings: parameter_bindings,
                        environment: Environment::default(),
                    })));
                }
                self.sequence(env_arguments, Step::Expr, fuel)?;
                self.tasks.push(Step::Expr(callee));
            }
            K::NominalType { .. }
            | K::CheckTypeAlias { .. }
            | K::PlannedFunctionInstance { .. }
            | K::NormalFormType { .. }
            | K::ApplyNormalForm { .. } => {
                self.tasks.push(Step::Inputs(expr));
                if let K::ApplyNormalForm { callee, .. } = &expr.kind {
                    self.tasks.push(Step::Expr(callee));
                }
            }
            K::EffectHandle {
                computation,
                handlers,
                ..
            } => {
                charge(fuel, handlers.len())?;
                for handler in handlers.iter().rev() {
                    self.sequence(&handler.arms, Step::Arm, fuel)?;
                }
                self.tasks.push(Step::Expr(computation));
            }
            _ => {
                if let K::Assign { place, .. }
                | K::BitAssign { place, .. }
                | K::BitUpdate { place, .. } = &expr.kind
                {
                    self.read(place.root, dependencies, fuel)?;
                }
                crate::visit::append_expression_children(expr, &mut self.tasks, fuel)?;
            }
        }
        Ok(())
    }

    fn statement(&mut self, expr: &'a Expr, fuel: &mut usize) -> Result<(), Error> {
        charge(fuel, 1)?;
        match &expr.kind {
            K::Let { binding, value } => {
                self.tasks.push(Step::Bind(*binding));
                self.tasks.push(Step::Expr(value));
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                if let Some(otherwise) = otherwise {
                    self.tasks
                        .push(Step::Otherwise(self.current.scope.mark(), otherwise));
                }
                self.tasks.extend(guard.as_deref().map(Step::Expr));
                self.tasks.push(Step::Pattern(pattern));
                self.tasks.push(Step::Expr(value));
            }
            _ => self.tasks.push(Step::Expr(expr)),
        }
        Ok(())
    }

    fn pattern(&mut self, pattern: &'a P, fuel: &mut usize) -> Result<(), Error> {
        charge(fuel, 1)?;
        match pattern {
            P::CodeEvidence { outputs } => {
                charge(fuel, outputs.len())?;
                self.current.scope.extend(outputs.iter().map(|(id, _)| *id));
            }
            P::Observe { binding, inner } => {
                if self.current.scope.insert(*binding) {
                    self.tasks.push(Step::Remove(*binding));
                }
                self.tasks.push(Step::Pattern(inner));
            }
            P::Binding { binding, inner } => {
                self.tasks.push(Step::Bind(*binding));
                self.tasks.push(Step::Pattern(inner));
            }
            P::Then { first, value, next } => {
                self.tasks.push(Step::Pattern(next));
                self.tasks.push(Step::Expr(value));
                self.tasks.push(Step::Pattern(first));
            }
            P::Guard { pattern, condition } => {
                self.tasks.push(Step::Expr(condition));
                self.tasks.push(Step::Pattern(pattern));
            }
            P::Remap { pattern, bindings } => {
                self.tasks.push(Step::Remap(bindings));
                self.tasks.push(Step::Pattern(pattern));
            }
            P::Or(patterns) => {
                if let Some((first, rest)) = patterns.split_first() {
                    self.tasks.push(Step::AfterAlternative(Box::new(Branches {
                        rest,
                        common: Alternatives::default(),
                        mark: self.current.scope.mark(),
                    })));
                    self.tasks.push(Step::Pattern(first));
                }
            }
            P::OptionalPresent(inner) | P::ErrorResult { inner, .. } => {
                self.tasks.push(Step::Pattern(inner))
            }
            P::NominalType { fields, .. }
            | P::Record { fields, .. }
            | P::AnonymousRecord { fields, .. }
            | P::Variant { fields, .. }
            | P::NormalForm {
                arguments: fields, ..
            }
            | P::Tuple(fields)
            | P::TypeTuple(fields) => self.sequence(fields, Step::Pattern, fuel)?,
            P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
        }
        Ok(())
    }

    fn argument(&mut self, argument: &'a A, fuel: &mut usize) -> Result<(), Error> {
        charge(fuel, 1)?;
        match argument {
            A::Value(expr) | A::ListSpread(expr) => self.tasks.push(Step::Expr(expr)),
            A::List { items, .. } | A::Tuple { items, .. } => {
                self.sequence(items, Step::Argument, fuel)?
            }
            A::Block {
                statements, result, ..
            } => {
                self.scope();
                self.tasks.extend(result.as_deref().map(Step::Argument));
                self.sequence(statements, Step::Statement, fuel)?;
            }
            A::If {
                condition,
                then_input,
                else_input,
                ..
            } => {
                self.tasks.push(Step::Argument(else_input));
                self.tasks.push(Step::Argument(then_input));
                self.tasks.push(Step::Expr(condition));
            }
            A::Match { value, arms, .. } => {
                self.sequence(arms, Step::InputArm, fuel)?;
                self.tasks.push(Step::Expr(value));
            }
            A::Integer { .. } | A::Unit { .. } | A::Null { .. } => {}
        }
        Ok(())
    }
}
