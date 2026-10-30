//! Invocation-local value references. These handles are interpreter storage,
//! not source pointers: no pointee type, runtime ownership transfer, or Copy
//! protocol is involved. Every access revalidates its structural field path.
use super::{ErrorKind, Storage, Value};
use nir::program::FieldOwner;
use std::{fmt, sync::MutexGuard};

#[derive(Clone, PartialEq)]
pub struct ValueReference {
    pub(crate) storage: Storage,
    fields: Vec<(FieldOwner, usize)>,
    rebind: bool,
}

impl ValueReference {
    /// Promote a binding to shared invocation storage without copying its value.
    pub(super) fn for_slot(value: &mut Value) -> Self {
        match value {
            Value::ComptimeReference(reference) => reference.clone(),
            Value::Storage(storage) => Self::root(storage.clone()),
            _ => {
                let storage = Storage::new(std::mem::replace(value, Value::Unit));
                *value = Value::Storage(storage.clone());
                Self::root(storage)
            }
        }
    }

    pub(crate) fn root(storage: Storage) -> Self {
        Self {
            storage,
            fields: Vec::new(),
            rebind: true,
        }
    }

    pub(crate) fn projected(
        storage: Storage,
        fields: &[(FieldOwner, usize)],
        fuel: &mut u64,
    ) -> Result<Self, ErrorKind> {
        charge(fields, fuel)?;
        Ok(Self {
            storage,
            fields: fields.to_vec(),
            rebind: true,
        })
    }

    pub(crate) fn borrow(&self, fuel: &mut u64) -> Result<Contents<'_>, ErrorKind> {
        charge(&self.fields, fuel)?;
        let guard = self
            .storage
            .0
            .try_lock()
            .map_err(|_| ErrorKind::InvalidValueReference)?;
        locate(&guard, &self.fields)?;
        Ok(Contents {
            guard,
            fields: &self.fields,
        })
    }

    /// Resolve after RHS evaluation and replace only the selected field. A bad
    /// path or exhausted budget leaves the entire original value unchanged.
    pub(crate) fn replace(&self, value: Value, fuel: &mut u64) -> Result<(), ErrorKind> {
        self.update(fuel, |target| {
            *target = value;
            Ok(())
        })
    }

    pub(crate) fn append_list_checked(
        &self,
        suffix: Value,
        fuel: &mut u64,
        check: impl FnOnce(&Value, u64) -> Result<(), ErrorKind>,
    ) -> Result<(), ErrorKind> {
        charge(&self.fields, fuel)?;
        let remaining = *fuel;
        self.update_unmetered(|target| {
            super::memory::append_list_checked(target, suffix, |value| check(value, remaining))
        })
    }

    fn update(
        &self,
        fuel: &mut u64,
        update: impl FnOnce(&mut Value) -> Result<(), ErrorKind>,
    ) -> Result<(), ErrorKind> {
        charge(&self.fields, fuel)?;
        self.update_unmetered(update)
    }

    fn update_unmetered(
        &self,
        update: impl FnOnce(&mut Value) -> Result<(), ErrorKind>,
    ) -> Result<(), ErrorKind> {
        if !self.rebind {
            return Err(ErrorKind::InvalidValueReference);
        }
        let mut guard = self
            .storage
            .0
            .try_lock()
            .map_err(|_| ErrorKind::InvalidValueReference)?;
        let mut target = &mut *guard;
        for (owner, index) in &self.fields {
            target = match (owner, target) {
                (
                    FieldOwner::Nominal(expected),
                    Value::Record {
                        declaration,
                        fields,
                        ..
                    },
                ) => {
                    if expected == declaration {
                        fields.get_mut(*index)
                    } else {
                        None
                    }
                }
                (FieldOwner::Anonymous, Value::AnonymousRecord(fields)) => {
                    fields.get_mut(*index).map(|(_, value)| value)
                }
                _ => None,
            }
            .ok_or(ErrorKind::InvalidValueReference)?;
        }
        update(target)
    }
}

fn charge(fields: &[(FieldOwner, usize)], fuel: &mut u64) -> Result<(), ErrorKind> {
    if fields.len() >= 128 {
        return Err(ErrorKind::DepthExhausted);
    }
    *fuel = fuel
        .checked_sub(fields.len() as u64 + 1)
        .ok_or(ErrorKind::StepsExhausted)?;
    Ok(())
}

fn locate<'a>(
    mut value: &'a Value,
    fields: &[(FieldOwner, usize)],
) -> Result<&'a Value, ErrorKind> {
    for (owner, index) in fields {
        value = match (owner, value) {
            (
                FieldOwner::Nominal(expected),
                Value::Record {
                    declaration,
                    fields,
                    ..
                },
            ) if expected == declaration => fields.get(*index),
            (FieldOwner::Anonymous, Value::AnonymousRecord(fields)) => {
                fields.get(*index).map(|(_, value)| value)
            }
            _ => None,
        }
        .ok_or(ErrorKind::InvalidValueReference)?;
    }
    Ok(value)
}

pub(super) struct Contents<'a> {
    guard: MutexGuard<'a, Value>,
    fields: &'a [(FieldOwner, usize)],
}
impl Contents<'_> {
    pub fn value(&self) -> &Value {
        // The path was checked under this guard and cannot change while held.
        locate(&self.guard, self.fields).expect("validated value reference path")
    }
}
impl fmt::Display for ValueReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(value-reference (fields")?;
        for (owner, index) in &self.fields {
            write!(f, " (field {owner} {index})")?;
        }
        write!(f, ") (rebind {}))", self.rebind)
    }
}
impl fmt::Debug for ValueReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl ValueReference {
    pub(crate) fn project(
        &self,
        fields: &[(FieldOwner, usize)],
        fuel: &mut u64,
    ) -> Result<Self, ErrorKind> {
        let length = self
            .fields
            .len()
            .checked_add(fields.len())
            .ok_or(ErrorKind::DepthExhausted)?;
        if length >= 128 {
            return Err(ErrorKind::DepthExhausted);
        }
        *fuel = fuel
            .checked_sub(length as u64 + 1)
            .ok_or(ErrorKind::StepsExhausted)?;
        let mut path = self.fields.clone();
        path.extend_from_slice(fields);
        Ok(Self {
            storage: self.storage.clone(),
            fields: path,
            rebind: self.rebind || !fields.is_empty(),
        })
    }
    pub(crate) fn prohibit_rebind(&mut self) {
        self.rebind = false;
    }

    pub(crate) fn metadata_work(&self) -> usize {
        self.fields.len() + 1
    }
}

impl crate::Interpreter<'_> {
    pub(super) fn write_comptime_place(
        &mut self,
        target: &mut Value,
        fields: &[(FieldOwner, usize)],
        value: Value,
        source: crate::SourceRange,
    ) -> Result<(), crate::Error> {
        let reference = match target {
            Value::ComptimeReference(reference) => Some(
                reference
                    .project(fields, &mut self.budget.steps)
                    .map_err(|kind| self.failure(kind, Some(source)))?,
            ),
            Value::Storage(storage) => Some(
                ValueReference::projected(storage.clone(), fields, &mut self.budget.steps)
                    .map_err(|kind| self.failure(kind, Some(source)))?,
            ),
            _ => None,
        };
        if let Some(reference) = reference {
            return reference
                .replace(value, &mut self.budget.steps)
                .map_err(|kind| self.failure(kind, Some(source)));
        }
        let mut target = target;
        for (owner, index) in fields {
            target = match (owner, target) {
                (
                    FieldOwner::Nominal(expected),
                    Value::Record {
                        declaration,
                        fields,
                        ..
                    },
                ) => {
                    if declaration == expected {
                        fields.get_mut(*index)
                    } else {
                        None
                    }
                }
                (FieldOwner::Anonymous, Value::AnonymousRecord(fields)) => {
                    fields.get_mut(*index).map(|(_, value)| value)
                }
                _ => None,
            }
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        }
        *target = value;
        Ok(())
    }

    pub(super) fn read_value_reference(
        &mut self,
        reference: &ValueReference,
        source: crate::SourceRange,
    ) -> Result<Value, crate::Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        let contents = reference
            .borrow(&mut self.budget.steps)
            .map_err(|kind| self.failure(kind, Some(source)))?;
        self.checkpoint_representation(contents.value(), source)
    }

    /// Preserve the base slot and the complete field path. Computed bases are
    /// evaluated once into private storage, without exposing a source pointer.
    pub(super) fn prepare_value_reference(
        &mut self,
        expr: &crate::Expr,
        environment: &mut crate::environment::Environment,
    ) -> Result<crate::Flow, crate::Error> {
        self.tick(Some(expr.source))?;
        let mut base = expr;
        let mut fields = Vec::new();
        loop {
            let (next, field) = match &base.kind {
                crate::K::Field {
                    value,
                    declaration,
                    index,
                } => (value.as_ref(), (FieldOwner::Nominal(*declaration), *index)),
                crate::K::AnonymousField { value, index, .. } => {
                    (value.as_ref(), (FieldOwner::Anonymous, *index))
                }
                _ => break,
            };
            self.tick(Some(base.source))?;
            if fields.len() >= 127 {
                return Err(self.failure(ErrorKind::DepthExhausted, Some(expr.source)));
            }
            fields.push(field);
            base = next;
        }
        fields.reverse();
        let reference = if let crate::K::Local(binding) = base.kind {
            let value = environment
                .get_mut(&binding)
                .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(base.source)))?;
            ValueReference::for_slot(value)
        } else {
            if matches!(base.kind, crate::K::ReadPointer(_)) {
                return Err(self.failure(ErrorKind::Unsupported, Some(base.source)));
            }
            match self.eval(base, environment)? {
                crate::Flow::Value(value) => ValueReference::root(Storage::new(value)),
                control => return Ok(control),
            }
        };
        let reference = self.project_value_reference(reference, &fields, expr.source)?;
        Ok(crate::Flow::Value(Value::ComptimeReference(reference)))
    }

    pub(super) fn project_value_reference(
        &mut self,
        reference: ValueReference,
        fields: &[(FieldOwner, usize)],
        source: crate::SourceRange,
    ) -> Result<ValueReference, crate::Error> {
        let reference = reference
            .project(fields, &mut self.budget.steps)
            .map_err(|kind| self.failure(kind, Some(source)))?;
        reference
            .borrow(&mut self.budget.steps)
            .map_err(|kind| self.failure(kind, Some(source)))?;
        Ok(reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Value {
        Value::AnonymousRecord(vec![
            ("first".into(), Value::Bool(false)),
            ("second".into(), Value::Bool(false)),
        ])
    }

    #[test]
    fn aliases_observe_replacement_without_changing_old_snapshots() {
        let storage = Storage::new(record());
        let root = ValueReference::root(storage.clone());
        let alias = root.clone();
        let snapshot = root.borrow(&mut 100).unwrap().value().clone();
        let first =
            ValueReference::projected(storage.clone(), &[(FieldOwner::Anonymous, 0)], &mut 100)
                .unwrap();
        let second =
            ValueReference::projected(storage, &[(FieldOwner::Anonymous, 1)], &mut 100).unwrap();
        second.replace(Value::Bool(true), &mut 100).unwrap();
        first.replace(Value::Bool(true), &mut 100).unwrap();
        assert_eq!(snapshot, record());
        assert_eq!(
            alias.borrow(&mut 100).unwrap().value(),
            &Value::AnonymousRecord(vec![
                ("first".into(), Value::Bool(true)),
                ("second".into(), Value::Bool(true))
            ])
        );
        assert_eq!(
            first.to_string(),
            "(value-reference (fields (field (anonymous-record) 0)) (rebind true))"
        );
    }

    #[test]
    fn rebind_permission_survives_forwarding_but_allows_field_updates() {
        let mut root = ValueReference::root(Storage::new(record()));
        root.prohibit_rebind();
        let forwarded = root.project(&[], &mut 100).unwrap();
        assert_eq!(
            forwarded.replace(Value::Bool(true), &mut 100),
            Err(ErrorKind::InvalidValueReference)
        );
        assert_eq!(root.borrow(&mut 100).unwrap().value(), &record());
        let field = forwarded
            .project(&[(FieldOwner::Anonymous, 0)], &mut 100)
            .unwrap();
        field.replace(Value::Bool(true), &mut 100).unwrap();
        assert_eq!(field.borrow(&mut 100).unwrap().value(), &Value::Bool(true));
        assert!(root.to_string().contains("(rebind false)"));
    }

    #[test]
    fn stale_projections_and_exhausted_work_never_modify_storage() {
        let storage = Storage::new(record());
        let root = ValueReference::root(storage.clone());
        let field =
            ValueReference::projected(storage, &[(FieldOwner::Anonymous, 0)], &mut 100).unwrap();
        assert_eq!(
            field.replace(Value::Bool(true), &mut 0),
            Err(ErrorKind::StepsExhausted)
        );
        assert_eq!(root.borrow(&mut 100).unwrap().value(), &record());
        root.replace(Value::Bool(false), &mut 100).unwrap();
        assert_eq!(
            field.replace(Value::Bool(true), &mut 100),
            Err(ErrorKind::InvalidValueReference)
        );
        assert_eq!(root.borrow(&mut 100).unwrap().value(), &Value::Bool(false));
        assert!(matches!(
            field.borrow(&mut 100),
            Err(ErrorKind::InvalidValueReference)
        ));
    }

    #[test]
    fn nested_paths_validate_owners_and_limits_before_writes() {
        let id = nir::DeclarationId {
            package: 1,
            local: 2,
        };
        let storage = Storage::new(Value::AnonymousRecord(vec![("nested".into(), record())]));
        let path = [(FieldOwner::Anonymous, 0), (FieldOwner::Anonymous, 1)];
        let selected = ValueReference::projected(storage.clone(), &path, &mut 100).unwrap();
        selected.replace(Value::Bool(true), &mut 100).unwrap();
        assert_eq!(
            selected.borrow(&mut 100).unwrap().value(),
            &Value::Bool(true)
        );
        let bad =
            ValueReference::projected(storage.clone(), &[(FieldOwner::Nominal(id), 0)], &mut 100)
                .unwrap();
        assert_eq!(
            bad.replace(Value::Bool(false), &mut 100),
            Err(ErrorKind::InvalidValueReference)
        );
        assert_eq!(
            selected.borrow(&mut 100).unwrap().value(),
            &Value::Bool(true)
        );
        assert!(matches!(
            ValueReference::projected(storage.clone(), &path, &mut 2),
            Err(ErrorKind::StepsExhausted)
        ));
        assert!(matches!(
            ValueReference::projected(storage, &vec![(FieldOwner::Anonymous, 0); 128], &mut 1000),
            Err(ErrorKind::DepthExhausted)
        ));
    }

    #[test]
    fn nominal_projection_revalidates_owner_after_root_replacement() {
        let id = nir::DeclarationId {
            package: 1,
            local: 3,
        };
        let record = |declaration| Value::Record {
            declaration,
            arguments: vec![],
            fields: vec![Value::Bool(false)],
        };
        let storage = Storage::new(record(id));
        let root = ValueReference::root(storage.clone());
        let field =
            ValueReference::projected(storage, &[(FieldOwner::Nominal(id), 0)], &mut 100).unwrap();
        field.replace(Value::Bool(true), &mut 100).unwrap();
        assert_eq!(field.borrow(&mut 100).unwrap().value(), &Value::Bool(true));
        let other = nir::DeclarationId {
            package: 2,
            local: 3,
        };
        root.replace(record(other), &mut 100).unwrap();
        assert_eq!(
            field.replace(Value::Bool(true), &mut 100),
            Err(ErrorKind::InvalidValueReference)
        );
        assert_eq!(root.borrow(&mut 100).unwrap().value(), &record(other));
    }

    #[test]
    fn reads_hold_storage_stable_and_reentrant_access_fails_without_waiting() {
        let root = ValueReference::root(Storage::new(Value::Bool(false)));
        let held = root.borrow(&mut 100).unwrap();
        assert_eq!(
            root.replace(Value::Bool(true), &mut 100),
            Err(ErrorKind::InvalidValueReference)
        );
        drop(held);
        root.replace(Value::Bool(true), &mut 100).unwrap();
        assert_eq!(root.borrow(&mut 100).unwrap().value(), &Value::Bool(true));
    }
}

#[cfg(test)]
mod invocation_tests {
    use super::*;
    use crate::{
        Budget, Expr, Flow, Interpreter, K, Literal, LocalId, SourceRange, environment::Environment,
    };
    use nir::{
        Primitive, Term,
        program::{Package, Place},
    };

    #[test]
    fn comptime_value_reference_assignment_rechecks_after_rhs_writes() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let binding = LocalId { owner: 1, index: 0 };
        let alias = LocalId { owner: 2, index: 0 };
        let source = SourceRange { start: 1, end: 2 };
        let boolean = Term::Primitive(Primitive::Bool);
        let literal = || Expr {
            source,
            ty: boolean.clone(),
            kind: K::Literal(Literal::Bool(true)),
        };
        let write = |index, value| Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Assign {
                place: Place {
                    root: binding,
                    fields: vec![(FieldOwner::Anonymous, index)],
                },
                value: Box::new(value),
            },
        };
        let rhs = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Block {
                statements: vec![write(1, literal())],
                result: Some(Box::new(literal())),
            },
        };
        let storage = Storage::new(Value::AnonymousRecord(vec![
            ("left".into(), Value::Bool(false)),
            ("right".into(), Value::Bool(false)),
        ]));
        let mut environment = Environment::new();
        environment.insert(binding, Value::Storage(storage.clone()));
        environment.insert(alias, Value::Storage(storage));
        let snapshot = vm.local_value(&environment, alias, Some(source)).unwrap();
        assert!(matches!(
            vm.eval(&write(0, rhs), &mut environment).unwrap(),
            Flow::Value(Value::Unit)
        ));
        assert_eq!(
            vm.local_value(&environment, alias, Some(source)).unwrap(),
            Value::AnonymousRecord(vec![
                ("left".into(), Value::Bool(true)),
                ("right".into(), Value::Bool(true))
            ])
        );
        assert_eq!(
            snapshot,
            Value::AnonymousRecord(vec![
                ("left".into(), Value::Bool(false)),
                ("right".into(), Value::Bool(false))
            ])
        );
        assert_eq!(
            vm.eval(&write(2, literal()), &mut environment)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidValueReference
        );
        assert_eq!(
            vm.local_value(&environment, alias, Some(source)).unwrap(),
            Value::AnonymousRecord(vec![
                ("left".into(), Value::Bool(true)),
                ("right".into(), Value::Bool(true))
            ])
        );
    }
}

#[cfg(test)]
mod call_tests {
    use crate::*;
    use nir::program::{ArgumentPassing, CallArgument, Function, FunctionPhase, Place};
    fn expr(kind: K) -> Expr {
        Expr {
            kind,
            ty: Term::Primitive(Primitive::Bool),
            source: SourceRange { start: 1, end: 2 },
        }
    }
    fn setter() -> Function {
        let binding = LocalId { owner: 2, index: 0 };
        Function {
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration: DeclarationId {
                package: 1,
                local: 2,
            },
            name: "set".into(),
            signature: Term::Function {
                parameters: vec![nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: Term::Primitive(Primitive::Bool),
                }],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            },
            parameters: vec![binding],
            defaults: vec![],
            body: expr(K::Block {
                statements: vec![Expr {
                    ty: Term::Primitive(Primitive::Void),
                    source: SourceRange { start: 1, end: 2 },
                    kind: K::Assign {
                        place: Place {
                            root: binding,
                            fields: vec![],
                        },
                        value: Box::new(expr(K::Literal(Literal::Bool(true)))),
                    },
                }],
                result: Some(Box::new(expr(K::Local(binding)))),
            }),
        }
    }
    fn call(target: &Function, passing: ArgumentPassing, value: Expr) -> Expr {
        expr(K::Call {
            callee: Box::new(Expr {
                ty: target.callable_type(),
                ..expr(K::Function(target.declaration))
            }),
            arguments: vec![CallArgument {
                parameter: 0,
                value,
                is_default: false,
                passing,
            }],
            parameter_bindings: target.parameters.clone(),
        })
    }
    #[test]
    fn explicit_reference_call_updates_the_callers_slot_but_value_call_does_not() {
        let target = setter();
        let package = Package {
            functions: vec![target.clone()],
            ..Default::default()
        };
        let binding = LocalId { owner: 1, index: 0 };
        for passing in [ArgumentPassing::Value, ArgumentPassing::ComptimeReference] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut environment = Environment::new();
            environment.insert(binding, Value::Bool(false));
            let call = call(&target, passing, expr(K::Local(binding)));
            assert!(matches!(
                vm.eval(&call, &mut environment).unwrap(),
                Flow::Value(Value::Bool(true))
            ));
            assert_eq!(
                vm.local_value(&environment, binding, None).unwrap(),
                Value::Bool(passing == ArgumentPassing::ComptimeReference)
            );
            assert_eq!(
                vm.events()
                    .iter()
                    .filter(|event| matches!(event, Event::EnterFunction(_)))
                    .count(),
                1
            );
        }
    }
    #[test]
    fn forwarded_references_keep_identity_and_computed_receivers_evaluate_once() {
        let target = setter();
        let mut forwarding = setter();
        forwarding.declaration.local = 3;
        forwarding.name = "forward".into();
        forwarding.parameters = vec![LocalId { owner: 3, index: 0 }];
        forwarding.body = call(
            &target,
            ArgumentPassing::ComptimeReference,
            expr(K::Local(forwarding.parameters[0])),
        );
        let package = Package {
            functions: vec![target.clone(), forwarding.clone()],
            ..Default::default()
        };
        nir::phases::validate(&package).unwrap();
        let binding = LocalId { owner: 1, index: 0 };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut environment = Environment::new();
        environment.insert(binding, Value::Bool(false));
        let forwarded = call(
            &forwarding,
            ArgumentPassing::ComptimeReference,
            expr(K::Local(binding)),
        );
        assert!(matches!(
            vm.eval(&forwarded, &mut environment).unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        assert_eq!(
            vm.local_value(&environment, binding, None).unwrap(),
            Value::Bool(true)
        );
        environment.insert(binding, Value::Bool(false));
        let before = vm
            .events()
            .iter()
            .filter(|event| matches!(event, Event::EnterFunction(_)))
            .count();
        let produced = call(&target, ArgumentPassing::Value, expr(K::Local(binding)));
        let temporary = call(&target, ArgumentPassing::ComptimeReference, produced);
        assert!(matches!(
            vm.eval(&temporary, &mut environment).unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        assert_eq!(
            vm.local_value(&environment, binding, None).unwrap(),
            Value::Bool(false)
        );
        let after = vm
            .events()
            .iter()
            .filter(|event| matches!(event, Event::EnterFunction(_)))
            .count();
        assert_eq!(after - before, 2);
    }

    #[test]
    fn projected_receivers_compose_through_calls_and_preserve_siblings() {
        let target = setter();
        let boolean = Term::Primitive(Primitive::Bool);
        let inner_type = Term::Record(vec![
            ("left".into(), boolean.clone()),
            ("right".into(), boolean.clone()),
        ]);
        let outer_type = Term::Record(vec![
            ("inner".into(), inner_type.clone()),
            ("tag".into(), boolean.clone()),
        ]);
        let mut forwarding = setter();
        forwarding.declaration.local = 3;
        forwarding.name = "forward_field".into();
        forwarding.parameters = vec![LocalId { owner: 3, index: 0 }];
        let Term::Function { parameters, .. } = &mut forwarding.signature else {
            unreachable!()
        };
        parameters[0].ty = inner_type.clone();
        let field = expr(K::AnonymousField {
            value: Box::new(Expr {
                ty: inner_type.clone(),
                ..expr(K::Local(forwarding.parameters[0]))
            }),
            name: "left".into(),
            index: 0,
        });
        forwarding.body = expr(K::Block {
            statements: vec![call(
                &target,
                ArgumentPassing::ComptimeReference,
                field.clone(),
            )],
            result: Some(Box::new(field)),
        });
        let package = Package {
            functions: vec![target.clone(), forwarding.clone()],
            ..Default::default()
        };
        nir::phases::validate(&package).unwrap();
        let binding = LocalId { owner: 1, index: 0 };
        let value = Value::AnonymousRecord(vec![
            (
                "inner".into(),
                Value::AnonymousRecord(vec![
                    ("left".into(), Value::Bool(false)),
                    ("right".into(), Value::Bool(false)),
                ]),
            ),
            ("tag".into(), Value::Bool(false)),
        ]);
        let receiver = Expr {
            ty: inner_type,
            ..expr(K::AnonymousField {
                value: Box::new(Expr {
                    ty: outer_type,
                    ..expr(K::Local(binding))
                }),
                name: "inner".into(),
                index: 0,
            })
        };
        let invocation = call(&forwarding, ArgumentPassing::ComptimeReference, receiver);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut environment = Environment::new();
        environment.insert(binding, value.clone());
        assert!(matches!(
            vm.eval(&invocation, &mut environment).unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        let got = vm.local_value(&environment, binding, None).unwrap();
        assert_eq!(
            got,
            Value::AnonymousRecord(vec![
                (
                    "inner".into(),
                    Value::AnonymousRecord(vec![
                        ("left".into(), Value::Bool(true)),
                        ("right".into(), Value::Bool(false))
                    ])
                ),
                ("tag".into(), Value::Bool(false)),
            ])
        );
        assert_ne!(got, value);
    }

    #[test]
    fn internal_references_cannot_be_frozen_or_read_by_runtime() {
        let reference = crate::ValueReference::root(Storage::new(Value::Bool(true)));
        let value = Value::ComptimeReference(reference.clone());
        assert!(
            value
                .clone()
                .into_normal_term(&mut comptime_runtime::Budget::default())
                .is_err()
        );
        assert_eq!(
            value.to_string(),
            "(value-reference (fields) (rebind true))"
        );
        let package = Package::default();
        let mut comptime = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            comptime
                .copy_representation(&value, SourceRange { start: 1, end: 2 }, 0)
                .unwrap_err()
                .kind,
            ErrorKind::Unsupported
        );

        let package = Package::default();
        let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            runtime
                .read_value_reference(&reference, SourceRange { start: 1, end: 2 })
                .unwrap_err()
                .kind,
            ErrorKind::PhaseMismatch
        );
    }

    #[test]
    fn reference_call_rejects_runtime_and_default_transport() {
        let target = setter();
        let package = Package {
            functions: vec![target.clone()],
            ..Default::default()
        };
        let mut call = call(
            &target,
            ArgumentPassing::ComptimeReference,
            expr(K::Literal(Literal::Bool(false))),
        );
        let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            runtime
                .eval(&call, &mut Environment::new())
                .unwrap_err()
                .kind,
            ErrorKind::InvalidCallPlan
        );
        let K::Call { arguments, .. } = &mut call.kind else {
            unreachable!()
        };
        arguments[0].is_default = true;
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.eval(&call, &mut Environment::new()).unwrap_err().kind,
            ErrorKind::InvalidCallPlan
        );
    }
}

#[cfg(test)]
mod append_tests {
    use super::*;
    use nir::Term;

    #[test]
    fn append_reuses_buffer_and_preserves_snapshots_and_permissions() {
        let mut values = Vec::with_capacity(16);
        values.push(Value::Type(Term::Primitive(nir::Primitive::Bool)));
        let buffer = values.as_ptr();
        let Value::List { ty, .. } = crate::test_type_list(vec![]) else {
            unreachable!()
        };
        let storage = Storage::new(Value::List { ty, values });
        let snapshot = storage.read().unwrap();
        let mut reference = ValueReference::root(storage.clone());
        let suffix = || crate::test_type_list(vec![Term::Primitive(nir::Primitive::Type)]);
        reference
            .append_list_checked(suffix(), &mut 100, |_, _| Ok(()))
            .unwrap();
        let Value::List { values: old, .. } = &snapshot else {
            unreachable!()
        };
        assert_eq!(old.len(), 1);
        {
            let contents = reference.borrow(&mut 100).unwrap();
            let Value::List { values, .. } = contents.value() else {
                unreachable!()
            };
            assert_eq!(values.len(), 2);
            assert_eq!(values.as_ptr(), buffer);
        }
        let updated = storage.read().unwrap();
        assert_eq!(
            reference.append_list_checked(suffix(), &mut 100, |_, _| Err(
                ErrorKind::RepresentationSizeLimit
            )),
            Err(ErrorKind::RepresentationSizeLimit)
        );
        assert_eq!(storage.read().unwrap(), updated);
        reference.prohibit_rebind();
        assert_eq!(
            reference.append_list_checked(suffix(), &mut 100, |_, _| panic!(
                "readonly receiver must not mutate"
            )),
            Err(ErrorKind::InvalidValueReference)
        );
        assert_eq!(storage.read().unwrap(), updated);
    }
}
