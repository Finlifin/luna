//! Invocation-local value references. These handles are interpreter storage,
//! not source pointers: no pointee type, runtime ownership transfer, or Copy
//! protocol is involved. Every access revalidates its structural field path.
use super::{ErrorKind, Storage, Value};
use nir::program::FieldOwner;
use std::{fmt, sync::MutexGuard};

#[derive(Clone, PartialEq)]
pub(super) struct ValueReference {
    storage: Storage,
    fields: Vec<(FieldOwner, usize)>,
}

impl ValueReference {
    pub fn root(storage: Storage) -> Self {
        Self {
            storage,
            fields: Vec::new(),
        }
    }

    pub fn projected(
        storage: Storage,
        fields: &[(FieldOwner, usize)],
        fuel: &mut u64,
    ) -> Result<Self, ErrorKind> {
        charge(fields, fuel)?;
        Ok(Self {
            storage,
            fields: fields.to_vec(),
        })
    }

    pub fn borrow(&self, fuel: &mut u64) -> Result<Contents<'_>, ErrorKind> {
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
    pub fn replace(&self, value: Value, fuel: &mut u64) -> Result<(), ErrorKind> {
        charge(&self.fields, fuel)?;
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
        *target = value;
        Ok(())
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
        f.write_str("))")
    }
}
impl fmt::Debug for ValueReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
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
            "(value-reference (fields (field (anonymous-record) 0)))"
        );
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
