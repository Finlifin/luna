//! Interpreter storage identity. Pointers retain weak targets, not owners. The
//! source memory verifier remains responsible for proving accesses statically.
use super::{ErrorKind, Value};
use nir::{DeclarationId, Term};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex, Weak},
};

#[derive(Clone)]
pub struct Storage(pub(crate) Arc<Mutex<Value>>, Arc<Mutex<Responsibility>>);

/// Runtime ownership metadata, independent of payload bits. Children default
/// to the last whole-subtree installation/transfer; a parent's construction
/// token is independent of subsequent moves out of its fields.
#[derive(Clone)]
struct Responsibility {
    constructed: bool,
    descendants: bool,
    children: BTreeMap<usize, Responsibility>,
}
impl Responsibility {
    fn new(present: bool) -> Self {
        Self {
            constructed: present,
            descendants: present,
            children: BTreeMap::new(),
        }
    }
    fn position(&mut self, path: &[usize]) -> &mut Self {
        let mut current = self;
        for index in path {
            let default = current.descendants;
            current = current
                .children
                .entry(*index)
                .or_insert_with(|| Self::new(default));
        }
        current
    }
    fn whole(&self) -> bool {
        self.constructed && self.descendants && self.children.values().all(Self::whole)
    }
    fn finish_ancestors(&mut self, path: &[usize], arities: &[usize]) {
        for depth in (0..path.len()).rev() {
            let parent = self.position(&path[..depth]);
            if (0..arities[depth]).all(|index| {
                parent
                    .children
                    .get(&index)
                    .map_or(parent.descendants, Self::whole)
            }) {
                *parent = Self::new(true);
            }
        }
    }
}
pub(crate) struct CopyContents<'a> {
    _responsibility: std::sync::MutexGuard<'a, Responsibility>,
    value: std::sync::MutexGuard<'a, Value>,
}
impl CopyContents<'_> {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }
}

impl fmt::Debug for Storage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Storage")
    }
}
impl PartialEq for Storage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Storage {
    pub fn new(value: Value) -> Self {
        Self(
            Arc::new(Mutex::new(value)),
            Arc::new(Mutex::new(Responsibility::new(true))),
        )
    }
    pub(crate) fn contents_for_copy(
        &self,
        fuel: &mut u64,
        max_depth: usize,
    ) -> Result<CopyContents<'_>, ErrorKind> {
        // Keep ownership and payload stable throughout borrowed copying. A
        // recursive owned-storage cycle fails rather than waiting on itself.
        let responsibility = self.1.try_lock().map_err(|_| ErrorKind::InvalidPointer)?;
        let mut pending = vec![(&*responsibility, 0usize)];
        while let Some((node, depth)) = pending.pop() {
            *fuel = fuel.checked_sub(1).ok_or(ErrorKind::StepsExhausted)?;
            if depth >= max_depth.min(128) {
                return Err(ErrorKind::DepthExhausted);
            }
            if !node.constructed || !node.descendants {
                return Err(ErrorKind::InvalidPointer);
            }
            // Charge width before growing the work list, including absent
            // payload work when a large ownership tree exhausts the budget.
            *fuel = fuel
                .checked_sub(
                    u64::try_from(node.children.len()).map_err(|_| ErrorKind::StepsExhausted)?,
                )
                .ok_or(ErrorKind::StepsExhausted)?;
            pending.extend(node.children.values().map(|child| (child, depth + 1)));
        }
        let value = self.0.try_lock().map_err(|_| ErrorKind::InvalidPointer)?;
        Ok(CopyContents {
            _responsibility: responsibility,
            value,
        })
    }
    pub fn read(&self) -> Result<Value, ErrorKind> {
        Ok(self
            .0
            .lock()
            .map_err(|_| ErrorKind::InvalidPointer)?
            .clone())
    }
    pub(crate) fn pointer(&self, pointee: Term, fields: Vec<Projection>) -> PointerValue {
        PointerValue {
            target: Arc::downgrade(&self.0),
            responsibility: Arc::downgrade(&self.1),
            ownership_guards: vec![],
            pointee,
            fields,
        }
    }
    /// Resolve an elaborated assignment without dropping its nominal owner or
    /// anonymous field-label checks when switching to shared owner storage.
    pub(crate) fn assignment_pointer(
        &self,
        pointee: Term,
        fields: &[(nir::program::FieldOwner, usize)],
    ) -> Result<PointerValue, ErrorKind> {
        let value = self.0.lock().map_err(|_| ErrorKind::InvalidPointer)?;
        let mut target = &*value;
        let mut projections = vec![];
        for (owner, index) in fields {
            match (owner, target) {
                (
                    nir::program::FieldOwner::Nominal(expected),
                    Value::Record {
                        declaration,
                        fields,
                        ..
                    },
                ) if expected == declaration => {
                    projections.push(Projection::Record(*declaration, *index));
                    target = fields.get(*index).ok_or(ErrorKind::Type)?;
                }
                (nir::program::FieldOwner::Anonymous, Value::AnonymousRecord(fields)) => {
                    let (name, value) = fields.get(*index).ok_or(ErrorKind::Type)?;
                    projections.push(Projection::AnonymousRecord(name.clone(), *index));
                    target = value;
                }
                _ => return Err(ErrorKind::Type),
            }
        }
        Ok(self.pointer(pointee, projections))
    }
    /// Host-side projection for verified cleanup; no source access permission is inferred.
    pub(crate) fn owned_pointer(
        &self,
        pointee: Term,
        path: &[usize],
    ) -> Result<PointerValue, ErrorKind> {
        self.owned_pointer_depth(pointee, path, 0)
    }
    fn owned_pointer_depth(
        &self,
        pointee: Term,
        path: &[usize],
        depth: usize,
    ) -> Result<PointerValue, ErrorKind> {
        if depth >= 128 {
            return Err(ErrorKind::DepthExhausted);
        }
        let storage = self.0.lock().map_err(|_| ErrorKind::InvalidPointer)?;
        let mut target = &*storage;
        let mut fields = vec![];
        for (position, index) in path.iter().enumerate() {
            if let Value::Storage(nested) = target {
                let nested = nested.clone();
                drop(storage);
                let mut pointer =
                    nested.owned_pointer_depth(pointee, &path[position..], depth + 1)?;
                pointer
                    .ownership_guards
                    .push((Arc::downgrade(&self.1), path[..position].to_vec()));
                return Ok(pointer);
            }
            match target {
                Value::Newtype { ty, value } if *index == 0 => {
                    fields.push(Projection::Newtype(ty.clone()));
                    target = value;
                }
                Value::Record {
                    declaration,
                    fields: values,
                    ..
                } => {
                    fields.push(Projection::Record(*declaration, *index));
                    target = values.get(*index).ok_or(ErrorKind::InvalidPointer)?;
                }
                Value::Variant {
                    declaration,
                    index: tag,
                    fields: values,
                    ..
                } => {
                    fields.push(Projection::Variant(*declaration, *tag, *index));
                    target = values.get(*index).ok_or(ErrorKind::InvalidPointer)?;
                }
                Value::AnonymousRecord(values) => {
                    let (name, value) = values.get(*index).ok_or(ErrorKind::InvalidPointer)?;
                    fields.push(Projection::AnonymousRecord(name.clone(), *index));
                    target = value;
                }
                Value::Tuple(values) => {
                    fields.push(Projection::Tuple(*index));
                    target = values.get(*index).ok_or(ErrorKind::InvalidPointer)?;
                }
                Value::Closure(closure) => {
                    let binding = *closure
                        .capture_order
                        .get(*index)
                        .ok_or(ErrorKind::InvalidPointer)?;
                    fields.push(Projection::Capture(binding));
                    target = closure
                        .environment
                        .get(&binding)
                        .ok_or(ErrorKind::InvalidPointer)?;
                }
                _ => return Err(ErrorKind::Unsupported),
            }
        }
        if let Value::Storage(nested) = target {
            let nested = nested.clone();
            drop(storage);
            let mut pointer = nested.owned_pointer_depth(pointee, &[], depth + 1)?;
            pointer
                .ownership_guards
                .push((Arc::downgrade(&self.1), path.to_vec()));
            return Ok(pointer);
        }
        Ok(PointerValue {
            target: Arc::downgrade(&self.0),
            responsibility: Arc::downgrade(&self.1),
            ownership_guards: vec![],
            pointee,
            fields,
        })
    }
}
#[derive(Clone, PartialEq)]
pub(crate) enum Projection {
    /// Cleanup-only descent; ordinary source field projection cannot create it.
    Newtype(Term),
    Record(DeclarationId, usize),
    Variant(DeclarationId, usize, usize),
    Tuple(usize),
    AnonymousRecord(String, usize),
    Capture(nir::program::LocalId),
}
#[derive(Clone)]
pub struct PointerValue {
    target: Weak<Mutex<Value>>,
    responsibility: Weak<Mutex<Responsibility>>,
    // Reaching shared capture storage does not erase the old enclosing owner's
    // responsibility. A moved closure can leave these same cells alive under a
    // new owner; cleanup through its former owner must still see absence.
    ownership_guards: Vec<(Weak<Mutex<Responsibility>>, Vec<usize>)>,
    pub(crate) pointee: Term,
    fields: Vec<Projection>,
}
impl PointerValue {
    pub(crate) fn copy_metadata_work(&self) -> Option<usize> {
        let mut work = self.fields.len().checked_add(self.ownership_guards.len())?;
        for field in &self.fields {
            if let Projection::AnonymousRecord(name, _) = field {
                work = work.checked_add(name.len())?;
            }
            if let Projection::Newtype(ty) = field {
                let mut fuel = 100_000;
                nir::type_computations::charge_term(ty, &mut fuel).ok()?;
                work = work.checked_add(100_000 - fuel)?;
            }
        }
        for (_, path) in &self.ownership_guards {
            work = work.checked_add(path.len())?;
        }
        Some(work)
    }
}
impl PartialEq for PointerValue {
    fn eq(&self, other: &Self) -> bool {
        self.target.ptr_eq(&other.target)
            && self.fields == other.fields
            && self.pointee == other.pointee
    }
}
impl fmt::Debug for PointerValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Display for PointerValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(pointer-value (pointee {}) (live {}) (fields",
            self.pointee,
            self.target.strong_count() != 0
        )?;
        for field in &self.fields {
            match field {
                Projection::Record(declaration, index) => {
                    write!(f, " (field {declaration} {index})")?
                }
                Projection::Variant(declaration, tag, index) => {
                    write!(f, " (variant-field {declaration} {tag} {index})")?
                }
                Projection::Tuple(index) => write!(f, " (tuple-field {index})")?,
                Projection::Newtype(id) => write!(f, " (newtype-representation {id})")?,
                Projection::AnonymousRecord(name, index) => write!(
                    f,
                    " (record-field {} {index})",
                    nir::program::Literal::String(name.clone())
                )?,
                Projection::Capture(binding) => write!(f, " (capture-field {binding})")?,
            }
        }
        f.write_str("))")
    }
}
impl PointerValue {
    fn with_target<T>(
        &self,
        operation: impl FnOnce(&mut Value) -> Result<T, ErrorKind>,
    ) -> Result<T, ErrorKind> {
        self.with_position(|value, _, _| operation(value))
    }
    fn with_position<T>(
        &self,
        operation: impl FnOnce(&mut Value, &[usize], &[usize]) -> Result<T, ErrorKind>,
    ) -> Result<T, ErrorKind> {
        let storage = self.target.upgrade().ok_or(ErrorKind::ExpiredPointer)?;
        let mut value = storage.lock().map_err(|_| ErrorKind::InvalidPointer)?;
        let mut target = &mut *value;
        let mut path = Vec::with_capacity(self.fields.len());
        let mut arities = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            arities.push(match &*target {
                Value::Record { fields, .. }
                | Value::Variant { fields, .. }
                | Value::Tuple(fields) => fields.len(),
                Value::AnonymousRecord(fields) => fields.len(),
                Value::Closure(closure) => closure.capture_order.len(),
                Value::Newtype { .. } => 1,
                _ => return Err(ErrorKind::InvalidPointer),
            });
            path.push(match field {
                Projection::Newtype(_) => 0,
                Projection::Record(_, index)
                | Projection::Variant(_, _, index)
                | Projection::AnonymousRecord(_, index)
                | Projection::Tuple(index) => *index,
                Projection::Capture(binding) => {
                    let Value::Closure(closure) = target else {
                        return Err(ErrorKind::InvalidPointer);
                    };
                    closure
                        .capture_order
                        .iter()
                        .position(|b| b == binding)
                        .ok_or(ErrorKind::InvalidPointer)?
                }
            });
            target = match field {
                Projection::Newtype(expected) => {
                    let Value::Newtype { ty, value } = target else {
                        return Err(ErrorKind::InvalidPointer);
                    };
                    if ty != expected {
                        return Err(ErrorKind::InvalidPointer);
                    }
                    value.as_mut()
                }
                Projection::Record(expected, index) => {
                    let Value::Record {
                        declaration,
                        fields,
                        ..
                    } = target
                    else {
                        return Err(ErrorKind::InvalidPointer);
                    };
                    if expected != declaration {
                        return Err(ErrorKind::InvalidPointer);
                    }
                    fields.get_mut(*index).ok_or(ErrorKind::InvalidPointer)?
                }
                Projection::Variant(expected, tag, index) => {
                    let Value::Variant {
                        declaration,
                        index: actual,
                        fields,
                        ..
                    } = target
                    else {
                        return Err(ErrorKind::InvalidPointer);
                    };
                    if expected != declaration || tag != actual {
                        return Err(ErrorKind::InvalidPointer);
                    }
                    fields.get_mut(*index).ok_or(ErrorKind::InvalidPointer)?
                }
                Projection::AnonymousRecord(expected, index) => {
                    let Value::AnonymousRecord(fields) = target else {
                        return Err(ErrorKind::InvalidPointer);
                    };
                    let (name, value) = fields.get_mut(*index).ok_or(ErrorKind::InvalidPointer)?;
                    if name != expected {
                        return Err(ErrorKind::InvalidPointer);
                    }
                    value
                }
                Projection::Tuple(index) => {
                    let Value::Tuple(fields) = target else {
                        return Err(ErrorKind::InvalidPointer);
                    };
                    fields.get_mut(*index).ok_or(ErrorKind::InvalidPointer)?
                }
                Projection::Capture(binding) => {
                    let Value::Closure(closure) = target else {
                        return Err(ErrorKind::InvalidPointer);
                    };
                    if !closure.capture_order.contains(binding) {
                        return Err(ErrorKind::InvalidPointer);
                    }
                    Arc::make_mut(closure)
                        .environment
                        .get_mut(binding)
                        .ok_or(ErrorKind::InvalidPointer)?
                }
            };
        }
        operation(target, &path, &arities)
    }
    fn with_responsibility<T>(
        &self,
        operation: impl FnOnce(&mut Value, &mut Responsibility) -> Result<T, ErrorKind>,
    ) -> Result<T, ErrorKind> {
        self.with_position(|value, path, _| {
            let responsibility = self
                .responsibility
                .upgrade()
                .ok_or(ErrorKind::ExpiredPointer)?;
            let mut responsibility = responsibility
                .lock()
                .map_err(|_| ErrorKind::InvalidPointer)?;
            operation(value, responsibility.position(path))
        })
    }
    pub(crate) fn is_constructed(&self) -> Result<bool, ErrorKind> {
        if !self.ownership_guards_live()? {
            return Ok(false);
        }
        self.with_responsibility(|_, responsibility| Ok(responsibility.constructed))
    }
    fn ownership_guards_live(&self) -> Result<bool, ErrorKind> {
        for (owner, path) in self.ownership_guards.iter().rev() {
            let owner = owner.upgrade().ok_or(ErrorKind::ExpiredPointer)?;
            let mut owner = owner.lock().map_err(|_| ErrorKind::InvalidPointer)?;
            if !owner.position(path).constructed {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(crate) fn transfer(&self) -> Result<(), ErrorKind> {
        if !self.ownership_guards_live()? {
            return Err(ErrorKind::InvalidCallPlan);
        }
        self.with_responsibility(|_, responsibility| {
            if !responsibility.whole() {
                return Err(ErrorKind::InvalidCallPlan);
            }
            *responsibility = Responsibility::new(false);
            Ok(())
        })
    }
    pub(crate) fn release(&self) -> Result<(), ErrorKind> {
        if !self.ownership_guards_live()? {
            return Err(ErrorKind::InvalidCallPlan);
        }
        self.with_responsibility(|value, responsibility| {
            if !responsibility.constructed {
                return Err(ErrorKind::InvalidCallPlan);
            }
            *value = Value::Unit;
            *responsibility = Responsibility::new(false);
            Ok(())
        })
    }
    pub(crate) fn read(&self) -> Result<Value, ErrorKind> {
        self.with_target(|value| Ok(value.clone()))
    }
    pub(crate) fn write(
        &self,
        value: Value,
        check: impl FnOnce(&Value) -> bool,
    ) -> Result<(), ErrorKind> {
        self.with_position(|target, path, arities| {
            if !check(target) {
                return Err(ErrorKind::Type);
            }
            let responsibility = self
                .responsibility
                .upgrade()
                .ok_or(ErrorKind::ExpiredPointer)?;
            let mut responsibility = responsibility
                .lock()
                .map_err(|_| ErrorKind::InvalidPointer)?;
            *target = value;
            *responsibility.position(path) = Responsibility::new(true);
            responsibility.finish_ancestors(path, arities);
            Ok(())
        })
    }
    pub(crate) fn project_anonymous(
        &self,
        name: &str,
        index: usize,
        pointee: Term,
    ) -> Result<Self, ErrorKind> {
        self.with_target(|target| match target {
            Value::AnonymousRecord(fields)
                if fields.get(index).is_some_and(|(label, _)| label == name) =>
            {
                Ok(())
            }
            _ => Err(ErrorKind::InvalidPointer),
        })?;
        let mut pointer = self.clone();
        pointer
            .fields
            .push(Projection::AnonymousRecord(name.to_owned(), index));
        pointer.pointee = pointee;
        Ok(pointer)
    }
    pub(crate) fn project(
        &self,
        declaration: DeclarationId,
        index: usize,
        pointee: Term,
    ) -> Result<Self, ErrorKind> {
        self.with_target(|target| match target {
            Value::Record {
                declaration: actual,
                fields,
                ..
            } if *actual == declaration && index < fields.len() => Ok(()),
            _ => Err(ErrorKind::InvalidPointer),
        })?;
        let mut pointer = self.clone();
        pointer.fields.push(Projection::Record(declaration, index));
        pointer.pointee = pointee;
        Ok(pointer)
    }
}

#[cfg(test)]
mod record_storage_tests {
    use super::*;
    fn boolean() -> Term {
        Term::Primitive(nir::Primitive::Bool)
    }
    fn nested() -> Value {
        Value::AnonymousRecord(vec![
            ("first".into(), Value::Bool(true)),
            (
                "child".into(),
                Value::AnonymousRecord(vec![("second".into(), Value::Bool(false))]),
            ),
        ])
    }
    #[test]
    fn record_owned_paths_read_write_and_retire_only_the_selected_field() {
        let storage = Storage::new(nested());
        let first = storage.owned_pointer(boolean(), &[0]).unwrap();
        let second = storage.owned_pointer(boolean(), &[1, 0]).unwrap();
        assert_eq!(second.read().unwrap(), Value::Bool(false));
        second
            .write(Value::Bool(true), |value| matches!(value, Value::Bool(_)))
            .unwrap();
        assert_eq!(first.read().unwrap(), Value::Bool(true));
        assert_eq!(second.read().unwrap(), Value::Bool(true));
        second.write(Value::Unit, |_| true).unwrap();
        assert_eq!(first.read().unwrap(), Value::Bool(true));
        assert_eq!(
            second.write(Value::Bool(false), |value| matches!(value, Value::Bool(_))),
            Err(ErrorKind::Type)
        );
        assert_eq!(second.read().unwrap(), Value::Unit);
        assert_eq!(
            storage.owned_pointer(boolean(), &[2]).unwrap_err(),
            ErrorKind::InvalidPointer
        );
    }
    #[test]
    fn record_paths_validate_labels_after_root_replacement_and_do_not_keep_owners_alive() {
        let storage = Storage::new(Value::AnonymousRecord(vec![(
            "flag".into(),
            Value::Bool(true),
        )]));
        let pointer = storage.owned_pointer(boolean(), &[0]).unwrap();
        let root = storage
            .owned_pointer(Term::Record(vec![("flag".into(), boolean())]), &[])
            .unwrap();
        root.write(
            Value::AnonymousRecord(vec![("other".into(), Value::Bool(false))]),
            |_| true,
        )
        .unwrap();
        assert_eq!(pointer.read(), Err(ErrorKind::InvalidPointer));
        root.write(
            Value::AnonymousRecord(vec![("flag".into(), Value::Bool(false))]),
            |_| true,
        )
        .unwrap();
        assert_eq!(pointer.read().unwrap(), Value::Bool(false));
        drop(storage);
        assert_eq!(pointer.read(), Err(ErrorKind::ExpiredPointer));
    }
    #[test]
    fn record_cleanup_paths_follow_nested_storage_identity() {
        let child = Storage::new(Value::AnonymousRecord(vec![(
            "flag".into(),
            Value::Bool(true),
        )]));
        let outer = Storage::new(Value::AnonymousRecord(vec![(
            "child".into(),
            Value::Storage(child.clone()),
        )]));
        let pointer = outer.owned_pointer(boolean(), &[0, 0]).unwrap();
        pointer
            .write(Value::Bool(false), |value| matches!(value, Value::Bool(_)))
            .unwrap();
        assert_eq!(
            child.read().unwrap(),
            Value::AnonymousRecord(vec![("flag".into(), Value::Bool(false))])
        );
        drop(outer);
        assert_eq!(pointer.read().unwrap(), Value::Bool(false));
        drop(child);
        assert_eq!(pointer.read(), Err(ErrorKind::ExpiredPointer));
    }
}

#[cfg(test)]
mod responsibility_tests {
    use super::*;
    fn pointer(storage: &Storage, path: &[usize]) -> PointerValue {
        storage
            .owned_pointer(Term::Primitive(nir::Primitive::Void), path)
            .unwrap()
    }
    #[test]
    fn unit_payload_is_live_until_its_responsibility_is_transferred() {
        let storage = Storage::new(Value::Unit);
        let owner = pointer(&storage, &[]);
        assert!(owner.is_constructed().unwrap());
        owner.transfer().unwrap();
        assert_eq!(owner.read().unwrap(), Value::Unit);
        assert!(!owner.is_constructed().unwrap());
        assert_eq!(owner.transfer(), Err(ErrorKind::InvalidCallPlan));
        assert_eq!(owner.release(), Err(ErrorKind::InvalidCallPlan));
        owner.write(Value::Unit, |_| true).unwrap();
        assert!(owner.is_constructed().unwrap());
        owner.release().unwrap();
        assert!(!owner.is_constructed().unwrap());
    }
    #[test]
    fn parent_token_survives_partial_moves_and_whole_transfer_requires_all_fields() {
        let storage = Storage::new(Value::Tuple(vec![Value::Unit, Value::Unit]));
        let parent = pointer(&storage, &[]);
        let first = pointer(&storage, &[0]);
        let second = pointer(&storage, &[1]);
        second.transfer().unwrap();
        assert!(parent.is_constructed().unwrap());
        assert!(first.is_constructed().unwrap());
        assert!(!second.is_constructed().unwrap());
        assert_eq!(parent.transfer(), Err(ErrorKind::InvalidCallPlan));
        second.write(Value::Unit, |_| true).unwrap();
        parent.transfer().unwrap();
        assert!(!first.is_constructed().unwrap());
        assert!(!second.is_constructed().unwrap());
    }
    #[test]
    fn reinstalling_all_fields_reconstructs_ancestors_but_not_before() {
        let storage = Storage::new(Value::Tuple(vec![
            Value::Unit,
            Value::Tuple(vec![Value::Unit]),
        ]));
        let parent = pointer(&storage, &[]);
        let first = pointer(&storage, &[0]);
        let nested = pointer(&storage, &[1]);
        let leaf = pointer(&storage, &[1, 0]);
        parent.transfer().unwrap();
        leaf.write(Value::Unit, |_| true).unwrap();
        assert!(nested.is_constructed().unwrap());
        assert!(!parent.is_constructed().unwrap());
        assert!(!first.is_constructed().unwrap());
        first.write(Value::Unit, |_| true).unwrap();
        assert!(parent.is_constructed().unwrap());
        parent.transfer().unwrap();
    }
    #[test]
    fn aliases_and_nested_storage_share_tokens_but_new_owners_do_not() {
        let captured = Storage::new(Value::Tuple(vec![Value::Unit]));
        let environment = Storage::new(Value::Tuple(vec![Value::Storage(captured.clone())]));
        let local = pointer(&captured, &[0]);
        let through_environment = pointer(&environment, &[0, 0]);
        let fresh = Storage::new(local.read().unwrap());
        local.transfer().unwrap();
        assert!(!through_environment.is_constructed().unwrap());
        assert!(pointer(&fresh, &[]).is_constructed().unwrap());
        through_environment.write(Value::Unit, |_| true).unwrap();
        assert!(local.is_constructed().unwrap());
        drop(environment);
        drop(captured);
        assert_eq!(local.is_constructed(), Err(ErrorKind::ExpiredPointer));
    }
    #[test]
    fn failed_writes_and_invalid_paths_preserve_tokens() {
        let storage = Storage::new(Value::Tuple(vec![Value::Bool(true)]));
        let leaf = pointer(&storage, &[0]);
        leaf.transfer().unwrap();
        assert_eq!(
            leaf.write(Value::Bool(false), |_| false),
            Err(ErrorKind::Type)
        );
        assert!(!leaf.is_constructed().unwrap());
        assert_eq!(leaf.read().unwrap(), Value::Bool(true));
        assert!(
            storage
                .owned_pointer(Term::Primitive(nir::Primitive::Void), &[1])
                .is_err()
        );
        assert!(pointer(&storage, &[]).is_constructed().unwrap());
    }
    #[test]
    fn enclosing_transfer_hides_shared_cells_only_from_the_previous_owner() {
        let cell = Storage::new(Value::Unit);
        let middle = Storage::new(Value::Tuple(vec![Value::Storage(cell.clone())]));
        let outer = Storage::new(Value::Tuple(vec![Value::Storage(middle.clone())]));
        let previous = pointer(&outer, &[0, 0]);
        let next = Storage::new(middle.read().unwrap());
        pointer(&middle, &[]).transfer().unwrap();
        assert!(!previous.is_constructed().unwrap());
        assert_eq!(previous.release(), Err(ErrorKind::InvalidCallPlan));
        assert!(pointer(&cell, &[]).is_constructed().unwrap());
        let current = pointer(&next, &[0]);
        assert!(current.is_constructed().unwrap());
        current.release().unwrap();
        assert!(!pointer(&cell, &[]).is_constructed().unwrap());
    }
}

#[cfg(test)]
mod copy_contents_tests {
    use super::*;
    #[test]
    fn representation_copy_bounds_ownership_tree_before_borrowing_payload() {
        let storage = Storage::new(Value::Tuple(vec![Value::Bool(true); 64]));
        {
            let mut responsibility = storage.1.lock().unwrap();
            for index in 0..64 {
                responsibility
                    .children
                    .insert(index, Responsibility::new(true));
            }
        }
        let mut fuel = 8;
        assert!(matches!(
            storage.contents_for_copy(&mut fuel, 128),
            Err(ErrorKind::StepsExhausted)
        ));
        assert_eq!(fuel, 7);
        let mut fuel = 1000;
        let contents = storage.contents_for_copy(&mut fuel, 128).unwrap();
        let Value::Tuple(fields) = contents.value() else {
            panic!("tuple")
        };
        assert_eq!(fields.len(), 64);
        assert!(storage.0.try_lock().is_err());
        assert!(storage.1.try_lock().is_err());
        drop(contents);
        assert!(storage.0.try_lock().is_ok());
        assert!(storage.1.try_lock().is_ok());
        storage
            .owned_pointer(Term::Primitive(nir::Primitive::Bool), &[0])
            .unwrap()
            .transfer()
            .unwrap();
        assert!(matches!(
            storage.contents_for_copy(&mut fuel, 128),
            Err(ErrorKind::InvalidPointer)
        ));
    }
}

#[cfg(test)]
mod newtype_storage_tests {
    use super::*;
    fn ty(argument: nir::Primitive) -> Term {
        Term::Nominal {
            declaration: DeclarationId {
                package: 7,
                local: 1,
            },
            arguments: vec![Term::Primitive(argument)],
        }
    }
    #[test]
    fn cleanup_path_is_private_and_checks_the_complete_wrapper_identity() {
        let storage = Storage::new(Value::Newtype {
            ty: ty(nir::Primitive::Bool),
            value: Box::new(Value::Bool(true)),
        });
        let payload = storage
            .owned_pointer(Term::Primitive(nir::Primitive::Bool), &[0])
            .unwrap();
        assert_eq!(payload.read().unwrap(), Value::Bool(true));
        assert!(
            storage
                .owned_pointer(Term::Primitive(nir::Primitive::Bool), &[1])
                .is_err()
        );
        assert!(
            storage
                .assignment_pointer(
                    Term::Primitive(nir::Primitive::Bool),
                    &[(
                        nir::program::FieldOwner::Nominal(DeclarationId {
                            package: 7,
                            local: 1
                        }),
                        0
                    )]
                )
                .is_err()
        );
        let root = storage
            .owned_pointer(ty(nir::Primitive::Bool), &[])
            .unwrap();
        root.write(
            Value::Newtype {
                ty: ty(nir::Primitive::Char),
                value: Box::new(Value::Char('x')),
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(payload.read(), Err(ErrorKind::InvalidPointer));
    }
}
