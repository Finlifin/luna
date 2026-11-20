//! Conservative program-contract rejection reachability, independent of memory
//! footprints, type propositions and error-result ABI. Unknown calls and
//! unverified implicit retirement may reject.
use super::pattern_facts::{bounded_outer_guards, constant_bool, known_pattern};
use nir::{
    DeclarationId, Primitive, Term,
    program::{Expr, ExprKind as K, FunctionPhase, Package, Pattern},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Instance {
    declaration: DeclarationId,
    arguments: Arc<[Term]>,
}
#[derive(Clone)]
struct Substitution(Arc<nir::DeclarationSubstitution<'static>>);
impl Substitution {
    fn new(
        parameters: &[DeclarationId],
        domains: &[Term],
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<Self, &'static str> {
        let substitution =
            nir::DeclarationSubstitution::bounded(parameters, domains, arguments, &[], fuel)
                .map_err(|_| "invalid rejection declaration arguments or exhausted budget")?;
        let owned = substitution
            .into_owned(fuel)
            .map_err(|_| "rejection substitution ownership budget exhausted")?;
        Ok(Self(Arc::new(owned)))
    }
    fn ty(&self, ty: &Term, fuel: &mut usize) -> Result<Term, &'static str> {
        self.0
            .instantiate_bounded(ty, fuel)
            .map_err(|_| "invalid rejection type substitution or exhausted budget")
    }
}
fn instance(
    callee: &Expr,
    context: Option<&Substitution>,
    fuel: &mut usize,
) -> Result<Option<Instance>, &'static str> {
    let (declaration, arguments) = match &callee.kind {
        K::Function(id) => (*id, &[][..]),
        K::FunctionInstance {
            declaration,
            arguments,
        } => (*declaration, arguments.as_slice()),
        _ => return Ok(None),
    };
    let mut actual = Vec::new();
    for argument in arguments {
        actual.push(if let Some(context) = context {
            context.ty(argument, fuel)?
        } else {
            super::memory::bound_type(argument, fuel)
                .map_err(|_| "rejection type budget exhausted")?;
            argument.clone()
        });
    }
    Ok(Some(Instance {
        declaration,
        arguments: actual.into(),
    }))
}
enum Work<'a> {
    Expr(&'a Expr, Substitution),
    Pattern(&'a Pattern, Substitution),
    Function(Instance),
}

/// One immutable package, one analysis transaction. No caller assumptions or
/// specialized type equivalences enter this property. Explicit type arguments
/// are substituted structurally and are part of every cached instance key.
/// Every query must use the same fixed cleanup policy; its selection evidence
/// is part of this transaction, not a globally reusable property of the body.
#[derive(Default)]
pub(crate) struct Memo {
    completed: HashMap<Instance, bool>,
}

enum MemoStorage<'a> {
    Owned(Memo),
    Borrowed(&'a mut Memo),
}

impl MemoStorage<'_> {
    fn as_ref(&self) -> &Memo {
        match self {
            Self::Owned(memo) => memo,
            Self::Borrowed(memo) => memo,
        }
    }

    fn as_mut(&mut self) -> &mut Memo {
        match self {
            Self::Owned(memo) => memo,
            Self::Borrowed(memo) => memo,
        }
    }
}

pub(super) struct Cache<'a> {
    package: Option<&'a Package>,
    declarations: std::rc::Rc<super::declarations::Metadata<'a>>,
    memo: MemoStorage<'a>,
}
impl<'a> Cache<'a> {
    pub(super) fn new(package: Option<&'a Package>) -> Self {
        Self::with_declarations(std::rc::Rc::new(super::declarations::Metadata::new(
            package,
        )))
    }
    pub(super) fn with_declarations(
        declarations: std::rc::Rc<super::declarations::Metadata<'a>>,
    ) -> Self {
        Self::with_declarations_and_memo(declarations, None)
    }
    pub(super) fn with_declarations_and_memo(
        declarations: std::rc::Rc<super::declarations::Metadata<'a>>,
        memo: Option<&'a mut Memo>,
    ) -> Self {
        Self {
            package: declarations.package(),
            declarations,
            memo: memo.map_or_else(
                || MemoStorage::Owned(Memo::default()),
                MemoStorage::Borrowed,
            ),
        }
    }
    pub(super) fn memo_mut(&mut self) -> &mut Memo {
        self.memo.as_mut()
    }
    #[cfg(test)]
    pub(super) fn may_reject(
        &mut self,
        callee: &Expr,
        fuel: &mut usize,
    ) -> Result<bool, &'static str> {
        self.may_reject_with_cleanup(callee, fuel, &mut |_, _| Ok(true))
    }
    pub(super) fn may_reject_with_cleanup(
        &mut self,
        callee: &Expr,
        fuel: &mut usize,
        cleanup: &mut dyn FnMut(&Term, &mut usize) -> Result<bool, String>,
    ) -> Result<bool, &'static str> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("rejection summary budget exhausted")?;
        let Some(root) = instance(callee, None, fuel)? else {
            return Ok(true);
        };
        self.query(root, fuel, cleanup)
    }
    pub(super) fn function_may_reject_with_cleanup(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        fuel: &mut usize,
        cleanup: &mut dyn FnMut(&Term, &mut usize) -> Result<bool, String>,
    ) -> Result<bool, &'static str> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("rejection summary budget exhausted")?;
        for argument in arguments {
            super::memory::bound_type(argument, fuel)
                .map_err(|_| "rejection type budget exhausted")?;
        }
        self.query(
            Instance {
                declaration,
                arguments: arguments.to_vec().into(),
            },
            fuel,
            cleanup,
        )
    }
    fn query(
        &mut self,
        root: Instance,
        fuel: &mut usize,
        cleanup: &mut dyn FnMut(&Term, &mut usize) -> Result<bool, String>,
    ) -> Result<bool, &'static str> {
        if self.package.is_none() {
            return Ok(true);
        }
        if let Some(result) = self.memo.as_ref().completed.get(&root) {
            return Ok(*result);
        }
        let Some(definitions) = self.declarations.function_index(fuel)? else {
            return Ok(true);
        };
        let (result, visited) = scan(
            &self.declarations,
            &definitions,
            &self.memo.as_ref().completed,
            root.clone(),
            fuel,
            cleanup,
        )?;
        // On success with no rejection, every visited definition's reachable
        // subgraph was closed. A positive search only establishes its root.
        if !result {
            let completed = &mut self.memo.as_mut().completed;
            for id in visited {
                completed.insert(id, false);
            }
        }
        self.memo.as_mut().completed.insert(root, result);
        Ok(result)
    }
}

fn cleanup_rejection(
    ty: &Term,
    fuel: &mut usize,
    cleanup: &mut dyn FnMut(&Term, &mut usize) -> Result<bool, String>,
) -> Result<bool, &'static str> {
    let result = cleanup(ty, fuel);
    if *fuel == 0 {
        return Err("rejection cleanup budget exhausted");
    }
    Ok(result.unwrap_or(true))
}

fn scan<'a>(
    declarations: &super::declarations::Metadata<'a>,
    definitions: &BTreeMap<DeclarationId, usize>,
    completed: &HashMap<Instance, bool>,
    root: Instance,
    fuel: &mut usize,
    cleanup: &mut dyn FnMut(&Term, &mut usize) -> Result<bool, String>,
) -> Result<(bool, HashSet<Instance>), &'static str> {
    let mut work = vec![Work::Function(root)];
    let mut visited = HashSet::new();
    while let Some(item) = work.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("rejection summary budget exhausted")?;
        match item {
            Work::Function(id) => {
                if !visited.insert(id.clone()) {
                    continue;
                }
                if let Some(rejects) = completed.get(&id) {
                    if *rejects {
                        return Ok((true, visited));
                    }
                    continue;
                }
                if !definitions.contains_key(&id.declaration) {
                    return Ok((true, visited));
                }
                // Analyze the selected residual, including its contracts. Its
                // completed staging inputs remain in the cache key, but are
                // no longer substitutions for the residual body's telescope.
                let selected = declarations
                    .package()
                    .ok_or("missing rejection package")?
                    .runtime_body_checked(id.declaration, &id.arguments);
                let (function, body_arguments) = match selected {
                    Ok(selected) => selected,
                    Err("function type arguments have incorrect arity") => {
                        return Ok((true, visited));
                    }
                    Err(error) => return Err(error),
                };
                if function.phase != FunctionPhase::Runtime
                    || function.type_parameters.len() != body_arguments.len()
                {
                    return Ok((true, visited));
                }
                // An empty telescope needs no directory. Nonempty inputs
                // retain declaration-domain admission in the owned work item.
                let domains = declarations
                    .parameter_domains(&function.type_parameters, fuel)
                    .map_err(|_| "invalid rejection declaration metadata or exhausted budget")?;
                let context =
                    Substitution::new(&function.type_parameters, &domains, body_arguments, fuel)?;
                // Parameter ownership is acquired even when the body never
                // reads the parameter. Its exit cleanup can run user code.
                if let Term::Function { parameters, .. } = function.signature.callable_signature() {
                    for parameter in parameters {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("rejection parameter budget exhausted")?;
                        let ty = context.ty(&parameter.ty, fuel)?;
                        if !(matches!(ty, Term::Primitive(_))
                            && nir::program::intrinsic_pointer_value(&ty)
                            || matches!(ty, Term::Primitive(Primitive::Void | Primitive::Never)))
                        {
                            if cleanup_rejection(&ty, fuel, cleanup)? {
                                return Ok((true, visited));
                            }
                        }
                    }
                }
                for condition in function
                    .preconditions
                    .iter()
                    .chain(&function.postconditions)
                {
                    if constant_bool(condition, 0) != Some(true) {
                        return Ok((true, visited));
                    }
                }
                work.push(Work::Expr(&function.body, context.clone()));
            }
            Work::Pattern(pattern, context) => match pattern {
                Pattern::CodeEvidence { .. } => return Err("unresolved pattern-code evidence"),
                Pattern::OptionalAbsent | Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => (),
                Pattern::OptionalPresent(pattern)
                | Pattern::ErrorResult { inner: pattern, .. }
                | Pattern::Observe { inner: pattern, .. }
                | Pattern::Binding { inner: pattern, .. }
                | Pattern::Remap { pattern, .. } => {
                    work.push(Work::Pattern(pattern, context.clone()))
                }
                Pattern::Guard { pattern, condition } => {
                    work.push(Work::Pattern(pattern, context.clone()));
                    work.push(Work::Expr(condition, context.clone()));
                }
                Pattern::Then { first, value, next } => {
                    work.push(Work::Pattern(first, context.clone()));
                    work.push(Work::Expr(value, context.clone()));
                    work.push(Work::Pattern(next, context.clone()));
                }
                Pattern::Tuple(fields)
                | Pattern::TypeTuple(fields)
                | Pattern::Or(fields)
                | Pattern::NominalType { fields, .. }
                | Pattern::Record { fields, .. }
                | Pattern::AnonymousRecord { fields, .. }
                | Pattern::Variant { fields, .. }
                | Pattern::NormalForm {
                    arguments: fields, ..
                } => work.extend(
                    fields
                        .iter()
                        .map(|node| Work::Pattern(node, context.clone())),
                ),
            },
            Work::Expr(expr, context) => match &expr.kind {
                K::Unit
                | K::Literal(_)
                | K::Local(_)
                | K::Function(_)
                | K::FunctionInstance { .. }
                | K::TraitMethod { .. }
                | K::ComptimeValue(_)
                | K::StagedValue(_)
                | K::TypeValue(_)
                | K::Break(_)
                | K::Continue(_) => (),
                K::RequirePredicate(condition)
                | K::EnsurePredicate(condition)
                | K::CheckPredicate(condition) => {
                    if constant_bool(condition, 0) != Some(true) {
                        return Ok((true, visited));
                    }
                }
                K::Call {
                    callee, arguments, ..
                } => {
                    let Some(target) = instance(callee, Some(&context), fuel)? else {
                        return Ok((true, visited));
                    };
                    work.push(Work::Function(target));
                    work.extend(
                        arguments
                            .iter()
                            .map(|a| Work::Expr(&a.value, context.clone())),
                    );
                }
                K::Closure { captures, .. } => {
                    // Capturing an owner creates an eventual cleanup obligation,
                    // even when the closure body is never invoked.
                    if !captures.is_empty()
                        && cleanup_rejection(&context.ty(&expr.ty, fuel)?, fuel, cleanup)?
                    {
                        return Ok((true, visited));
                    }
                    work.extend(
                        captures
                            .iter()
                            .filter_map(|c| c.value.as_deref())
                            .map(|node| Work::Expr(node, context.clone())),
                    );
                }
                K::ErrorConvert { value, .. } | K::NewtypeConvert(value) => {
                    // Representation conversions are not contract rejection.
                    // The operand and resulting cleanup obligation still
                    // contribute to the invocation's behavior, including a
                    // newtype wrapper's own destructor.
                    if cleanup_rejection(&context.ty(&expr.ty, fuel)?, fuel, cleanup)? {
                        return Ok((true, visited));
                    }
                    work.push(Work::Expr(value, context.clone()));
                }
                K::Tuple(fields)
                | K::Object { fields, .. }
                | K::AnonymousRecord { fields }
                | K::Record { fields, .. }
                | K::Variant { fields, .. } => {
                    // Field initialization is not the whole behavior. The fixed
                    // cleanup world must certify implicit retirement as well.
                    if cleanup_rejection(&context.ty(&expr.ty, fuel)?, fuel, cleanup)? {
                        return Ok((true, visited));
                    }
                    work.extend(fields.iter().map(|node| Work::Expr(node, context.clone())));
                }
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    let mut live = true;
                    for statement in statements {
                        work.push(Work::Expr(statement, context.clone()));
                        if super::pattern_facts::cannot_complete(statement, fuel, 0)
                            .map_err(|_| "rejection control-flow budget exhausted")?
                        {
                            live = false;
                            break;
                        }
                    }
                    if live {
                        work.extend(
                            result
                                .as_deref()
                                .map(|node| Work::Expr(node, context.clone())),
                        );
                    }
                }
                K::Return(value) => work.extend(
                    value
                        .as_deref()
                        .map(|node| Work::Expr(node, context.clone())),
                ),
                K::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    work.push(Work::Expr(condition, context.clone()));
                    let known = constant_bool(condition, 0);
                    if known != Some(false) {
                        work.push(Work::Expr(then_branch, context.clone()));
                    }
                    if known != Some(true) {
                        work.extend(
                            else_branch
                                .as_deref()
                                .map(|node| Work::Expr(node, context.clone())),
                        );
                    }
                }
                K::And(left, right) | K::Or(left, right) => {
                    work.push(Work::Expr(left, context.clone()));
                    let skipped = context.ty(&expr.ty, fuel)? == Term::Primitive(Primitive::Bool)
                        && constant_bool(left, 0) == Some(matches!(expr.kind, K::Or(..)));
                    if !skipped {
                        work.push(Work::Expr(right, context.clone()));
                    }
                }
                K::LetPattern {
                    pattern,
                    value,
                    guard,
                    otherwise,
                } => {
                    work.push(Work::Pattern(pattern, context.clone()));
                    work.push(Work::Expr(value, context.clone()));
                    work.extend(
                        guard
                            .as_deref()
                            .map(|node| Work::Expr(node, context.clone())),
                    );
                    work.extend(
                        otherwise
                            .as_deref()
                            .map(|node| Work::Expr(node, context.clone())),
                    );
                }
                K::Match { value, arms } => {
                    work.push(Work::Expr(value, context.clone()));
                    for arm in arms {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("rejection arm budget exhausted")?;
                        let (pattern, mut guards) = bounded_outer_guards(&arm.pattern, fuel)
                            .map_err(|_| "rejection pattern guard budget exhausted")?;
                        guards.extend(arm.guard.as_ref());
                        let observations = nir::visit::pattern_observations(pattern, fuel)
                            .map_err(|_| "rejection pattern observation budget exhausted")?;
                        let known = known_pattern(pattern, value, 0);
                        if known == Some(false) {
                            continue;
                        }
                        work.extend(
                            observations
                                .into_iter()
                                .map(|node| Work::Expr(node, context.clone())),
                        );
                        let mut accepted = Some(true);
                        for guard in guards {
                            work.push(Work::Expr(guard, context.clone()));
                            match constant_bool(guard, 0) {
                                Some(false) => {
                                    accepted = Some(false);
                                    break;
                                }
                                None => accepted = None,
                                _ => {}
                            }
                        }
                        if accepted != Some(false) {
                            work.push(Work::Expr(&arm.body, context.clone()));
                        }
                        if known == Some(true) && accepted == Some(true) {
                            break;
                        }
                    }
                }
                K::AnonymousField { value, .. }
                | K::Field { value, .. }
                | K::Let { value, .. }
                | K::Assign { value, .. }
                | K::BitAssign { value, .. }
                | K::UnsafeScope(value)
                | K::Discard(value)
                | K::BitSlice { value, .. }
                | K::AddressOf(value)
                | K::ReadPointer(value)
                | K::PointerAnonymousField { pointer: value, .. }
                | K::PointerField { pointer: value, .. }
                | K::IntegerNegate(value)
                | K::FloatNegate(value)
                | K::BitConvert(value)
                | K::Snapshot(value)
                | K::TakeBits(value)
                | K::TextQuery { subject: value, .. }
                | K::IntegerRepresentation(value)
                | K::TryIntegerConvert(value)
                | K::ComptimeIntegerConvert(value)
                | K::SymbolName(value)
                | K::EnumName(value)
                | K::TypeKind { operand: value, .. }
                | K::TypeLayout { operand: value, .. }
                | K::Not(value)
                | K::Loop { body: value, .. } => work.push(Work::Expr(value, context.clone())),
                K::BitUpdate {
                    selections, value, ..
                }
                | K::BitWritePointer {
                    selections, value, ..
                } => {
                    // Range and storage legality are mandatory program/memory
                    // obligations, as for BitAssign and integer arithmetic.
                    // Rejection evidence must still inspect all computations.
                    *fuel = fuel
                        .checked_sub(selections.len())
                        .ok_or("rejection bit selection budget exhausted")?;
                    if let K::BitWritePointer { pointer, .. } = &expr.kind {
                        work.push(Work::Expr(pointer, context.clone()));
                    }
                    work.push(Work::Expr(value, context.clone()));
                    for input in selections
                        .iter()
                        .flat_map(nir::program::BitSelection::expressions)
                    {
                        work.push(Work::Expr(input, context.clone()));
                    }
                }
                K::BitWindow { value, start, end } => {
                    for input in [value, start, end] {
                        work.push(Work::Expr(input, context.clone()));
                    }
                }
                K::WritePointer {
                    pointer: left,
                    value: right,
                }
                | K::IntegerBinary { left, right, .. }
                | K::FloatBinary { left, right, .. }
                | K::ScalarLess(left, right)
                | K::ScalarGreater(left, right)
                | K::ScalarLessEqual(left, right)
                | K::ScalarGreaterEqual(left, right)
                | K::TemplateEqual(left, right)
                | K::ScalarEqual(left, right)
                | K::BitEqual(left, right)
                | K::BitIndex(left, right)
                | K::BitConcat(left, right) => {
                    work.push(Work::Expr(left, context.clone()));
                    work.push(Work::Expr(right, context.clone()));
                }
                // Computed type expressions and future nodes need their own
                // staging rules before proving absence of runtime rejection.
                _ => return Ok((true, visited)),
            },
        }
    }
    Ok((false, visited))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::{Function, Literal, SourceRange};
    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 0, local }
    }
    fn expr(kind: K) -> Expr {
        Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: Term::Primitive(Primitive::Void),
            kind,
        }
    }
    fn callee(local: u32) -> Expr {
        expr(K::Function(id(local)))
    }
    fn call(local: u32) -> Expr {
        expr(K::Call {
            callee: Box::new(callee(local)),
            arguments: vec![],
            parameter_bindings: vec![],
        })
    }
    fn block(statements: Vec<Expr>) -> Expr {
        expr(K::Block {
            statements,
            result: None,
        })
    }
    fn rejected() -> Expr {
        expr(K::CheckPredicate(Box::new(Expr {
            ty: Term::Primitive(Primitive::Bool),
            ..expr(K::Literal(Literal::Bool(false)))
        })))
    }
    fn function(local: u32, body: Expr) -> Function {
        Function {
            interface: nir::program::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                phase: FunctionPhase::Runtime,
                inline: false,
                pure: false,
                is_unsafe: false,
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: id(local),
                name: format!("f{local}"),
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(Primitive::Void)),
                },
                parameters: vec![],
                defaults: vec![],
            },
            body,
        }
    }
    #[test]
    fn error_tags_preserve_operand_and_cleanup_rejection_without_becoming_rejection() {
        for conversion in [
            nir::error_results::Conversion::Success,
            nir::error_results::Conversion::Failure,
        ] {
            for rejecting_operand in [false, true] {
                for rejecting_cleanup in [false, true] {
                    let boolean = Term::Primitive(Primitive::Bool);
                    let target = Term::error_result(vec![boolean.clone()], boolean.clone());
                    let literal = Expr {
                        ty: boolean.clone(),
                        ..expr(K::Literal(Literal::Bool(true)))
                    };
                    let operand = if rejecting_operand {
                        Expr {
                            ty: boolean,
                            ..expr(K::Block {
                                statements: vec![rejected()],
                                result: Some(Box::new(literal)),
                            })
                        }
                    } else {
                        literal
                    };
                    let body = Expr {
                        ty: target.clone(),
                        ..expr(K::ErrorConvert {
                            conversion,
                            value: Box::new(operand),
                        })
                    };
                    let mut definition = function(0, body);
                    definition.signature = Term::Function {
                        parameters: vec![],
                        result: Box::new(target),
                    };
                    let mut package = Package::default();
                    package.functions.push(definition);
                    let actual = Cache::new(Some(&package))
                        .may_reject_with_cleanup(&callee(0), &mut 10000, &mut |_, _| {
                            Ok(rejecting_cleanup)
                        })
                        .unwrap();
                    assert_eq!(actual, rejecting_operand || rejecting_cleanup);
                }
            }
        }
    }

    #[test]
    fn structural_mismatch_does_not_erase_prior_pattern_rejection() {
        for computation_first in [false, true] {
            let computation = Pattern::Then {
                first: Box::new(Pattern::Wild),
                value: Box::new(rejected()),
                next: Box::new(Pattern::Wild),
            };
            let mismatch = Pattern::Literal(Literal::Bool(false));
            let fields = if computation_first {
                vec![computation, mismatch]
            } else {
                vec![mismatch, computation]
            };
            let truth = Expr {
                ty: Term::Primitive(Primitive::Bool),
                ..expr(K::Literal(Literal::Bool(true)))
            };
            let body = expr(K::Match {
                value: Box::new(expr(K::Record {
                    declaration: id(99),
                    fields: vec![truth.clone(), truth],
                })),
                arms: vec![
                    nir::program::MatchArm {
                        pattern: Pattern::Record {
                            declaration: id(99),
                            fields,
                        },
                        guard: None,
                        body: expr(K::Unit),
                    },
                    nir::program::MatchArm {
                        pattern: Pattern::Wild,
                        guard: None,
                        body: expr(K::Unit),
                    },
                ],
            });
            let K::Match { value, arms } = &body.kind else {
                unreachable!()
            };
            assert_eq!(
                known_pattern(&arms[0].pattern, value, 0),
                if computation_first { None } else { Some(false) }
            );
            let mut package = Package::default();
            package.functions.push(function(0, body));
            // Skipping a pattern computation cannot certify the subject's Drop.
            assert!(
                Cache::new(Some(&package))
                    .may_reject(&callee(0), &mut 1000)
                    .unwrap()
            );
        }
    }

    #[test]
    fn pure_unknown_field_does_not_hide_a_later_structural_mismatch() {
        let truth = Expr {
            ty: Term::Primitive(Primitive::Bool),
            ..expr(K::Literal(Literal::Bool(true)))
        };
        let unknown = Expr {
            ty: Term::Primitive(Primitive::Bool),
            ..expr(K::Local(nir::program::LocalId { owner: 0, index: 0 }))
        };
        let body = expr(K::Match {
            value: Box::new(expr(K::Record {
                declaration: id(99),
                fields: vec![unknown, truth],
            })),
            arms: vec![
                nir::program::MatchArm {
                    pattern: Pattern::Record {
                        declaration: id(99),
                        fields: vec![
                            Pattern::Literal(Literal::Bool(false)),
                            Pattern::Literal(Literal::Bool(false)),
                        ],
                    },
                    guard: None,
                    body: rejected(),
                },
                nir::program::MatchArm {
                    pattern: Pattern::Wild,
                    guard: None,
                    body: expr(K::Unit),
                },
            ],
        });
        let K::Match { value, arms } = &body.kind else {
            unreachable!()
        };
        assert_eq!(known_pattern(&arms[0].pattern, value, 0), Some(false));
        let package = Package {
            functions: vec![function(0, body)],
            ..Default::default()
        };
        assert!(
            Cache::new(Some(&package))
                .may_reject(&callee(0), &mut 1000)
                .unwrap()
        );
    }

    #[test]
    fn cleanup_evidence_is_required_and_budget_failure_is_not_cached() {
        let owned = Term::Nominal {
            declaration: id(99),
            arguments: vec![],
        };
        let body = Expr {
            ty: owned.clone(),
            ..expr(K::Record {
                declaration: id(99),
                fields: vec![],
            })
        };
        let package = Package {
            functions: vec![function(0, body)],
            ..Default::default()
        };
        let mut cache = Cache::new(Some(&package));
        assert!(
            cache
                .may_reject_with_cleanup(&callee(0), &mut 1000, &mut |_, fuel| {
                    *fuel = 0;
                    Err("exhausted acquisition".into())
                })
                .is_err()
        );
        assert!(cache.memo.as_ref().completed.is_empty());
        assert!(
            !cache
                .may_reject_with_cleanup(&callee(0), &mut 1000, &mut |ty, _| {
                    assert_eq!(ty, &owned);
                    Ok(false)
                })
                .unwrap()
        );
        assert!(
            !cache
                .may_reject_with_cleanup(&callee(0), &mut 1, &mut |_, _| {
                    panic!("completed cleanup proof should be reused")
                })
                .unwrap()
        );
        assert!(
            Cache::new(Some(&package))
                .may_reject(&callee(0), &mut 1000)
                .unwrap()
        );
    }

    fn generic_callee(local: u32, arguments: Vec<Term>) -> Expr {
        expr(K::FunctionInstance {
            declaration: id(local),
            arguments,
        })
    }
    fn variable(local: u32) -> Term {
        Term::Rigid {
            declaration: id(local),
            sort: nir::Sort::Type,
        }
    }
    fn generic_function(local: u32, parameter: u32, body: Expr) -> Function {
        let mut f = function(local, body);
        f.type_parameters = vec![id(parameter)];
        f
    }
    fn generic_call(local: u32, argument: Term) -> Expr {
        expr(K::Call {
            callee: Box::new(generic_callee(local, vec![argument])),
            arguments: vec![],
            parameter_bindings: vec![],
        })
    }

    #[test]
    fn context_inputs_use_declared_sorts_and_keep_cleanup_cache_separate() {
        let signature = function(99, expr(K::Unit)).interface.signature;
        let context = Term::contextual_closure(
            Term::Rigid {
                declaration: id(10),
                sort: nir::Sort::ComptimeValue,
            },
            signature,
        )
        .unwrap();
        let mut f = generic_function(0, 10, expr(K::Unit));
        let Term::Function { parameters, .. } = &mut f.signature else {
            unreachable!()
        };
        parameters.push(nir::Parameter {
            sort: nir::Sort::Runtime,
            ty: context,
        });
        let package = Package {
            functions: vec![f],
            function_parameters: vec![nir::function_parameters::Declaration {
                declaration: id(0),
                parameters: vec![id(10)],
                domains: vec![Term::ClosureContextType],
            }],
            ..Default::default()
        };
        let mut cache = Cache::new(Some(&package));
        assert!(
            cache
                .may_reject(
                    &generic_callee(0, vec![Term::Primitive(Primitive::Bool)]),
                    &mut 1000
                )
                .is_err()
        );
        let mut calls = 0;
        for captured in [false, true, false, true] {
            let argument = Term::ClosureContextValue(if captured {
                vec![Term::Primitive(Primitive::Bool)]
            } else {
                vec![]
            });
            let rejects = cache
                .may_reject_with_cleanup(
                    &generic_callee(0, vec![argument]),
                    &mut 1000,
                    &mut |ty, _| {
                        calls += 1;
                        assert!(matches!(
                            ty,
                            Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. }
                        ));
                        Ok(matches!(ty, Term::Closure { .. }))
                    },
                )
                .unwrap();
            assert_eq!(rejects, captured);
        }
        assert_eq!(calls, 2);
    }

    #[test]
    fn cleanup_rejection_cache_separates_instantiations_in_both_orders() {
        let body = Expr {
            ty: Term::Nominal {
                declaration: id(90),
                arguments: vec![variable(10)],
            },
            ..expr(K::Record {
                declaration: id(90),
                fields: vec![],
            })
        };
        let package = Package {
            functions: vec![generic_function(0, 10, body)],
            ..Default::default()
        };
        for order in [
            [Primitive::Bool, Primitive::U8],
            [Primitive::U8, Primitive::Bool],
        ] {
            let mut cache = Cache::new(Some(&package));
            let mut queries = 0;
            for kind in order.into_iter().chain(order) {
                let callee = generic_callee(0, vec![Term::Primitive(kind)]);
                let result = cache
                    .may_reject_with_cleanup(&callee, &mut 1000, &mut |ty, _| {
                        queries += 1;
                        let Term::Nominal { arguments, .. } = ty else {
                            panic!("missing concrete owner")
                        };
                        assert!(matches!(arguments.as_slice(), [Term::Primitive(_)]));
                        Ok(arguments == &[Term::Primitive(Primitive::U8)])
                    })
                    .unwrap();
                assert_eq!(result, kind == Primitive::U8);
            }
            assert_eq!(queries, 2, "one cleanup proof per distinct instance");
        }
    }

    #[test]
    fn unused_generic_parameters_check_the_actual_owner_type() {
        let mut f = generic_function(0, 10, expr(K::Unit));
        let Term::Function { parameters, .. } = &mut f.signature else {
            unreachable!()
        };
        parameters.push(nir::Parameter {
            sort: nir::Sort::Runtime,
            ty: variable(10),
        });
        let package = Package {
            functions: vec![f],
            ..Default::default()
        };
        let mut cache = Cache::new(Some(&package));
        assert!(
            !cache
                .may_reject_with_cleanup(
                    &generic_callee(0, vec![Term::Primitive(Primitive::Bool)]),
                    &mut 1000,
                    &mut |_, _| { panic!("intrinsic scalar parameter has no owned cleanup query") }
                )
                .unwrap()
        );
        let owner = Term::Nominal {
            declaration: id(77),
            arguments: vec![],
        };
        let mut queries = 0;
        assert!(
            cache
                .may_reject_with_cleanup(
                    &generic_callee(0, vec![owner.clone()]),
                    &mut 1000,
                    &mut |ty, _| {
                        queries += 1;
                        assert_eq!(ty, &owner);
                        Ok(true)
                    }
                )
                .unwrap()
        );
        assert_eq!(queries, 1);
    }

    #[test]
    fn nested_generic_calls_substitute_without_unfolding_normal_form_heads() {
        let wrapped = Term::NormalForm {
            head: id(55),
            sort: nir::Sort::Type,
            arguments: vec![variable(10)],
        };
        let body = Expr {
            ty: Term::Nominal {
                declaration: id(90),
                arguments: vec![variable(11)],
            },
            ..expr(K::Record {
                declaration: id(90),
                fields: vec![],
            })
        };
        let package = Package {
            functions: vec![
                generic_function(0, 10, generic_call(1, wrapped)),
                generic_function(1, 11, body),
            ],
            ..Default::default()
        };
        let mut seen = vec![];
        let mut cache = Cache::new(Some(&package));
        assert!(
            !cache
                .may_reject_with_cleanup(
                    &generic_callee(0, vec![Term::Primitive(Primitive::Bool)]),
                    &mut 1000,
                    &mut |ty, _| {
                        seen.push(ty.clone());
                        Ok(false)
                    }
                )
                .unwrap()
        );
        assert_eq!(
            seen,
            [Term::Nominal {
                declaration: id(90),
                arguments: vec![Term::NormalForm {
                    head: id(55),
                    sort: nir::Sort::Type,
                    arguments: vec![Term::Primitive(Primitive::Bool)]
                }]
            }]
        );
    }

    #[test]
    fn selected_residual_controls_rejection_and_malformed_selection_fails() {
        for source_rejects in [false, true] {
            let body = |rejects| if rejects { rejected() } else { expr(K::Unit) };
            let source = generic_function(0, 10, body(source_rejects));
            let residual = function(0, body(!source_rejects));
            let arguments = vec![Term::Primitive(Primitive::Bool)];
            let selected = nir::program::RuntimeInstance {
                declaration: id(0),
                arguments: arguments.clone(),
                function: residual,
                cleanup_accesses: vec![],
            };
            let mut package = Package {
                functions: vec![source],
                runtime_instances: vec![selected.clone()],
                ..Default::default()
            };
            let target = generic_callee(0, arguments);
            assert_eq!(
                Cache::new(Some(&package))
                    .may_reject(&target, &mut 1000)
                    .unwrap(),
                !source_rejects,
            );
            package.runtime_instances.push(selected);
            assert!(
                Cache::new(Some(&package))
                    .may_reject(&target, &mut 1000)
                    .is_err()
            );
            package.runtime_instances.pop();
            package.runtime_instances[0]
                .function
                .preconditions
                .push(Expr {
                    ty: Term::Primitive(Primitive::Bool),
                    ..expr(K::Literal(Literal::Bool(false)))
                });
            assert!(
                Cache::new(Some(&package))
                    .may_reject(&target, &mut 1000)
                    .unwrap()
            );
        }
    }

    #[test]
    fn generic_recursion_distinguishes_closed_and_growing_instances() {
        let package = Package {
            functions: vec![
                generic_function(0, 10, generic_call(0, variable(10))),
                generic_function(
                    1,
                    11,
                    generic_call(1, Term::Tuple(vec![variable(11), variable(11)])),
                ),
            ],
            ..Default::default()
        };
        let mut cache = Cache::new(Some(&package));
        assert!(
            !cache
                .may_reject(
                    &generic_callee(0, vec![Term::Primitive(Primitive::Bool)]),
                    &mut 1000
                )
                .unwrap()
        );
        assert!(
            !cache
                .may_reject(
                    &generic_callee(0, vec![Term::Primitive(Primitive::Bool)]),
                    &mut 2
                )
                .unwrap()
        );
        let completed = cache.memo.as_ref().completed.len();
        assert!(
            cache
                .may_reject(
                    &generic_callee(1, vec![Term::Primitive(Primitive::Bool)]),
                    &mut 1000
                )
                .is_err()
        );
        assert_eq!(
            cache.memo.as_ref().completed.len(),
            completed,
            "exhausted expansion cannot certify a recursive component"
        );
        assert!(
            !cache
                .may_reject(
                    &generic_callee(0, vec![Term::Primitive(Primitive::U8)]),
                    &mut 1000
                )
                .unwrap()
        );
        assert!(
            cache.may_reject(&callee(0), &mut 1000).unwrap(),
            "missing type argument is not the closed instance"
        );
    }

    #[test]
    fn closed_recursive_component_is_reused_with_constant_query_work() {
        let package = Package {
            functions: vec![function(0, call(1)), function(1, call(0))],
            ..Default::default()
        };
        let mut cache = Cache::new(Some(&package));
        assert!(!cache.may_reject(&callee(0), &mut 100).unwrap());
        for local in [0, 1, 0, 1] {
            let mut budget = 1;
            assert!(!cache.may_reject(&callee(local), &mut budget).unwrap());
            assert_eq!(budget, 0);
        }
    }
    #[test]
    fn exhausted_search_does_not_publish_partial_negative_results() {
        let package = Package {
            functions: (0..30)
                .map(|i| function(i, if i == 29 { rejected() } else { call(i + 1) }))
                .collect(),
            ..Default::default()
        };
        let mut cache = Cache::new(Some(&package));
        assert!(cache.may_reject(&callee(0), &mut 40).is_err());
        assert!(cache.memo.as_ref().completed.is_empty());
        assert!(cache.may_reject(&callee(0), &mut 1000).unwrap());
        assert!(cache.may_reject(&callee(0), &mut 1).unwrap());
    }
    #[test]
    fn positive_root_does_not_poison_an_unrelated_visited_callee() {
        let package = Package {
            functions: vec![
                function(0, block(vec![call(1), call(2)])),
                function(1, rejected()),
                function(2, expr(K::Unit)),
            ],
            ..Default::default()
        };
        let mut cache = Cache::new(Some(&package));
        assert!(cache.may_reject(&callee(0), &mut 100).unwrap());
        assert!(!cache.may_reject(&callee(2), &mut 100).unwrap());
        assert!(cache.may_reject(&callee(1), &mut 100).unwrap());
    }
    #[test]
    fn new_package_analysis_does_not_reuse_old_definition_results() {
        let mut package = Package {
            functions: vec![function(0, expr(K::Unit))],
            ..Default::default()
        };
        assert!(
            !Cache::new(Some(&package))
                .may_reject(&callee(0), &mut 100)
                .unwrap()
        );
        package.functions[0].body = rejected();
        assert!(
            Cache::new(Some(&package))
                .may_reject(&callee(0), &mut 100)
                .unwrap()
        );
        package.functions.push(package.functions[0].clone());
        assert!(
            Cache::new(Some(&package))
                .may_reject(&callee(0), &mut 100)
                .unwrap_err()
                .contains("duplicate")
        );
    }

    #[test]
    fn dependent_domains_remain_admitted_in_owned_rejection_work() {
        let mut function = generic_function(0, 900, expr(K::Unit));
        function.type_parameters.push(id(901));
        let package = Package {
            functions: vec![function],
            function_parameters: vec![nir::function_parameters::Declaration {
                declaration: id(0),
                parameters: vec![id(900), id(901)],
                domains: vec![
                    Term::Primitive(Primitive::Type),
                    Term::Rigid {
                        declaration: id(900),
                        sort: nir::Sort::Type,
                    },
                ],
            }],
            ..Default::default()
        };
        let callee = generic_callee(
            0,
            vec![
                Term::Primitive(Primitive::Type),
                Term::Primitive(Primitive::U8),
            ],
        );
        let mut cache = Cache::new(Some(&package));
        assert!(cache.may_reject(&callee, &mut 1).is_err());
        assert!(cache.memo.as_ref().completed.is_empty());
        assert!(!cache.may_reject(&callee, &mut 1000).unwrap());
        let invalid = generic_callee(
            0,
            vec![
                Term::Primitive(Primitive::Bool),
                Term::Primitive(Primitive::U8),
            ],
        );
        assert!(cache.may_reject(&invalid, &mut 1000).is_err());
        assert_eq!(cache.memo.as_ref().completed.len(), 1);
        let valid = generic_callee(0, vec![Term::Primitive(Primitive::Bool), Term::Bool(true)]);
        assert!(!cache.may_reject(&valid, &mut 1000).unwrap());
        assert_eq!(cache.memo.as_ref().completed.len(), 2);
    }

    #[test]
    fn declaration_evidence_is_shared_across_queries_without_caching_failures() {
        let package = Package {
            functions: vec![
                generic_function(0, 900, expr(K::Unit)),
                generic_function(1, 901, expr(K::Unit)),
            ],
            alias_contracts: (100..356)
                .map(|local| nir::type_aliases::Contract {
                    declaration: id(local),
                    parameters: vec![],
                    parameter_types: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                })
                .collect(),
            ..Default::default()
        };
        let callee = |local| generic_callee(local, vec![Term::Primitive(Primitive::Bool)]);
        let mut cache = Cache::new(Some(&package));
        assert!(cache.may_reject(&callee(0), &mut 64).is_err());
        assert!(!cache.declarations.domains_prepared());
        assert!(cache.memo.as_ref().completed.is_empty());
        assert!(!cache.may_reject(&callee(0), &mut 10_000).unwrap());
        assert!(cache.declarations.domains_prepared());
        // This is a different root, so only the declaration directory is warm.
        assert_eq!(cache.memo.as_ref().completed.len(), 1);
        let mut fuel = 64;
        assert!(!cache.may_reject(&callee(1), &mut fuel).unwrap());
        assert!(fuel < 64);
        assert_eq!(cache.memo.as_ref().completed.len(), 2);
        assert!(
            Cache::new(Some(&package))
                .may_reject(&callee(1), &mut 64)
                .is_err()
        );

        let mut other_world = package.clone();
        other_world.functions[1].body = rejected();
        assert!(
            Cache::new(Some(&other_world))
                .may_reject(&callee(1), &mut 10_000)
                .unwrap()
        );
        assert!(!cache.may_reject(&callee(1), &mut 100).unwrap());
        other_world.alias_contracts[0].parameters = vec![id(900), id(900)];
        let mut invalid = Cache::new(Some(&other_world));
        for _ in 0..2 {
            assert!(invalid.may_reject(&callee(1), &mut 10_000).is_err());
            assert!(!invalid.declarations.domains_prepared());
            assert!(invalid.memo.as_ref().completed.is_empty());
        }
    }

    #[test]
    fn closed_calls_defer_sort_observation_until_a_generic_edge_is_reached() {
        let package = Package {
            functions: vec![
                function(0, expr(K::Unit)),
                function(1, generic_call(2, Term::Primitive(Primitive::Bool))),
                generic_function(2, 900, expr(K::Unit)),
            ],
            alias_contracts: (100..356)
                .map(|local| nir::type_aliases::Contract {
                    declaration: id(local),
                    parameters: vec![],
                    parameter_types: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                })
                .collect(),
            ..Default::default()
        };
        let mut cache = Cache::new(Some(&package));
        assert!(!cache.may_reject(&callee(0), &mut 64).unwrap());
        assert!(!cache.declarations.domains_prepared());
        assert!(cache.may_reject(&callee(1), &mut 64).is_err());
        assert!(!cache.declarations.domains_prepared());
        assert_eq!(cache.memo.as_ref().completed.len(), 1);
        assert!(!cache.may_reject(&callee(1), &mut 10_000).unwrap());
        assert!(cache.declarations.domains_prepared());
    }
}
