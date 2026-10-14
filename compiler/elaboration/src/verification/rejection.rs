//! Conservative program-contract rejection reachability, independent of memory
//! footprints, type propositions and error-result ABI. Unknown calls may reject.
use super::pattern_facts::constant_bool;
use nir::{
    DeclarationId, Primitive, Term,
    program::{Expr, ExprKind as K, Function, FunctionPhase, Package, Pattern},
};
use std::collections::{BTreeMap, BTreeSet};

enum Work<'a> {
    Expr(&'a Expr),
    Pattern(&'a Pattern),
    Function(DeclarationId),
}

/// One immutable package, one analysis transaction. No caller assumptions or
/// specialized type equivalences enter this conservative definition property.
pub(super) struct Cache<'a> {
    package: Option<&'a Package>,
    definitions: Option<BTreeMap<DeclarationId, &'a Function>>,
    completed: BTreeMap<DeclarationId, bool>,
}
impl<'a> Cache<'a> {
    pub(super) fn new(package: Option<&'a Package>) -> Self {
        Self {
            package,
            definitions: None,
            completed: BTreeMap::new(),
        }
    }
    pub(super) fn may_reject(
        &mut self,
        callee: &Expr,
        fuel: &mut usize,
    ) -> Result<bool, &'static str> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("rejection summary budget exhausted")?;
        let root = match &callee.kind {
            K::Function(id)
            | K::FunctionInstance {
                declaration: id, ..
            } => *id,
            _ => return Ok(true),
        };
        let Some(package) = self.package else {
            return Ok(true);
        };
        if let Some(result) = self.completed.get(&root) {
            return Ok(*result);
        }
        if self.definitions.is_none() {
            *fuel = fuel
                .checked_sub(package.functions.len())
                .ok_or("rejection summary budget exhausted")?;
            let mut definitions = BTreeMap::new();
            for function in &package.functions {
                if definitions.insert(function.declaration, function).is_some() {
                    return Err("duplicate function identity in rejection summary");
                }
            }
            self.definitions = Some(definitions);
        }
        let (result, visited) = scan(
            self.definitions.as_ref().unwrap(),
            &self.completed,
            root,
            fuel,
        )?;
        // On success with no rejection, every visited definition's reachable
        // subgraph was closed. A positive search only establishes its root.
        if !result {
            for id in visited {
                self.completed.insert(id, false);
            }
        }
        self.completed.insert(root, result);
        Ok(result)
    }
}

fn scan(
    definitions: &BTreeMap<DeclarationId, &Function>,
    completed: &BTreeMap<DeclarationId, bool>,
    root: DeclarationId,
    fuel: &mut usize,
) -> Result<(bool, BTreeSet<DeclarationId>), &'static str> {
    let mut work = vec![Work::Function(root)];
    let mut visited = BTreeSet::new();
    while let Some(item) = work.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("rejection summary budget exhausted")?;
        match item {
            Work::Function(id) => {
                if !visited.insert(id) {
                    continue;
                }
                if let Some(rejects) = completed.get(&id) {
                    if *rejects {
                        return Ok((true, visited));
                    }
                    continue;
                }
                let Some(function) = definitions.get(&id) else {
                    return Ok((true, visited));
                };
                if function.phase != FunctionPhase::Runtime {
                    return Ok((true, visited));
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
                work.push(Work::Expr(&function.body));
            }
            Work::Pattern(pattern) => match pattern {
                Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => (),
                Pattern::Binding { inner: pattern, .. } | Pattern::Remap { pattern, .. } => {
                    work.push(Work::Pattern(pattern))
                }
                Pattern::Guard { pattern, condition } => {
                    work.push(Work::Pattern(pattern));
                    work.push(Work::Expr(condition));
                }
                Pattern::Then { first, value, next } => {
                    work.push(Work::Pattern(first));
                    work.push(Work::Expr(value));
                    work.push(Work::Pattern(next));
                }
                Pattern::Tuple(fields)
                | Pattern::TypeTuple(fields)
                | Pattern::Or(fields)
                | Pattern::Record { fields, .. } | Pattern::AnonymousRecord { fields, .. }
                | Pattern::Variant { fields, .. }
                | Pattern::NormalForm {
                    arguments: fields, ..
                } => work.extend(fields.iter().map(Work::Pattern)),
            },
            Work::Expr(expr) => match &expr.kind {
                K::Unit
                | K::Literal(_)
                | K::Local(_)
                | K::Function(_)
                | K::FunctionInstance { .. }
                | K::TraitMethod { .. }
                | K::TypeValue(_) | K::TypeListValue(_)
                | K::Break(_)
                | K::Continue(_) => (),
                K::RequirePredicate(condition) | K::EnsurePredicate(condition) | K::CheckPredicate(condition) => {
                    if constant_bool(condition, 0) != Some(true) {
                        return Ok((true, visited));
                    }
                }
                K::Call {
                    callee, arguments, ..
                } => {
                    match &callee.kind {
                        K::Function(id)
                        | K::FunctionInstance {
                            declaration: id, ..
                        } => work.push(Work::Function(*id)),
                        _ => return Ok((true, visited)),
                    }
                    work.extend(arguments.iter().map(|a| Work::Expr(&a.value)));
                }
                K::Closure { captures, .. } => {
                    // Building a closure does not invoke its checks.
                    work.extend(
                        captures
                            .iter()
                            .filter_map(|c| c.value.as_deref())
                            .map(Work::Expr),
                    );
                }
                K::Tuple(fields) | K::AnonymousRecord { fields } | K::Record { fields, .. } | K::Variant { fields, .. } => {
                    work.extend(fields.iter().map(Work::Expr))
                }
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    work.extend(statements.iter().map(Work::Expr));
                    work.extend(result.as_deref().map(Work::Expr));
                }
                K::Return(value) => work.extend(value.as_deref().map(Work::Expr)),
                K::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    work.push(Work::Expr(condition));
                    let known = constant_bool(condition, 0);
                    if known != Some(false) {
                        work.push(Work::Expr(then_branch));
                    }
                    if known != Some(true) {
                        work.extend(else_branch.as_deref().map(Work::Expr));
                    }
                }
                K::And(left, right) | K::Or(left, right) => {
                    work.push(Work::Expr(left));
                    let skipped = expr.ty == Term::Primitive(Primitive::Bool)
                        && constant_bool(left, 0) == Some(matches!(expr.kind, K::Or(..)));
                    if !skipped {
                        work.push(Work::Expr(right));
                    }
                }
                K::LetPattern {
                    pattern,
                    value,
                    guard,
                    otherwise,
                } => {
                    work.push(Work::Pattern(pattern));
                    work.push(Work::Expr(value));
                    work.extend(guard.as_deref().map(Work::Expr));
                    work.extend(otherwise.as_deref().map(Work::Expr));
                }
                K::Match { value, arms } => {
                    work.push(Work::Expr(value));
                    for arm in arms {
                        work.push(Work::Pattern(&arm.pattern));
                        work.push(Work::Expr(&arm.body));
                        work.extend(arm.guard.as_ref().map(Work::Expr));
                    }
                }
                K::AnonymousField { value, .. } | K::Field { value, .. }
                | K::Let { value, .. }
                | K::Assign { value, .. }
                | K::BitAssign { value, .. }
                | K::Discard(value)
                | K::BitSlice { value, .. }
                | K::AddressOf(value)
                | K::ReadPointer(value)
                | K::PointerAnonymousField { pointer: value, .. }
                | K::PointerField { pointer: value, .. }
                | K::IntegerNegate(value)
                | K::FloatNegate(value)
                | K::BitConvert(value)
                | K::Snapshot(value) | K::TakeBits(value) | K::SymbolName(value) | K::EnumName(value)
                | K::Not(value)
                | K::Loop { body: value, .. } => work.push(Work::Expr(value)),
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
                | K::ScalarEqual(left, right)
                | K::BitEqual(left, right)
                | K::BitConcat(left, right) => {
                    work.push(Work::Expr(left));
                    work.push(Work::Expr(right));
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
    use nir::program::{Literal, SourceRange};
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
            phase: FunctionPhase::Runtime,
            inline: false, pure: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
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
            body,
            defaults: vec![],
        }
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
        assert!(cache.completed.is_empty());
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
}
