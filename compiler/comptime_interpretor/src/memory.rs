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
    /// Inspect readable owned contents without cloning them. The ownership and
    /// payload guards remain live throughout inspection. Recursive acquisition
    /// fails instead of deadlocking; callers account for their own traversal.
    pub fn inspect_owned<R>(
        &self,
        fuel: &mut u64,
        max_depth: usize,
        inspect: impl FnOnce(&Value, &mut u64) -> R,
    ) -> Result<R, ErrorKind> {
        let contents = self.contents_for_copy(fuel, max_depth)?;
        Ok(inspect(contents.value(), fuel))
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
                Value::Optional {
                    ty,
                    value: Some(value),
                } if *index == 0 => {
                    fields.push(Projection::Optional(ty.clone()));
                    target = value;
                }
                Value::ErrorResult { ty, error, value } if *index == 0 => {
                    fields.push(Projection::ErrorResult(ty.clone(), *error));
                    target = value;
                }
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
    /// Cleanup-only payload projection, guarded by result type and active branch.
    Optional(Term),
    ErrorResult(Term, Option<usize>),
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
            if let Projection::Optional(ty)
            | Projection::Newtype(ty)
            | Projection::ErrorResult(ty, _) = field
            {
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
                Projection::Optional(ty) => write!(f, " (optional-payload (type {ty}))")?,
                Projection::ErrorResult(ty, branch) => {
                    write!(f, " (error-payload (type {ty}) (branch ")?;
                    match branch {
                        None => f.write_str("success")?,
                        Some(index) => write!(f, "(error {index})")?,
                    };
                    f.write_str("))")?;
                }
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
    /// Borrow a live target without cloning its payload or granting write access.
    pub(crate) fn inspect<T>(&self, inspect: impl FnOnce(&Value) -> T) -> Result<T, ErrorKind> {
        self.with_target(|value| Ok(inspect(value)))
    }
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
                Value::Optional { value: Some(_), .. }
                | Value::ErrorResult { .. }
                | Value::Newtype { .. } => 1,
                _ => return Err(ErrorKind::InvalidPointer),
            });
            path.push(match field {
                Projection::Optional(_) | Projection::ErrorResult(..) | Projection::Newtype(_) => 0,
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
                Projection::Optional(expected) => {
                    let Value::Optional {
                        ty,
                        value: Some(value),
                    } = target
                    else {
                        return Err(ErrorKind::InvalidPointer);
                    };
                    if ty != expected {
                        return Err(ErrorKind::InvalidPointer);
                    }
                    value.as_mut()
                }
                Projection::ErrorResult(expected, branch) => {
                    let Value::ErrorResult { ty, error, value } = target else {
                        return Err(ErrorKind::InvalidPointer);
                    };
                    if ty != expected || error != branch {
                        return Err(ErrorKind::InvalidPointer);
                    }
                    value.as_mut()
                }
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
    /// Extend an owned list buffer without changing any previously copied value.
    /// Validation runs under the storage lock and must not execute user code.
    /// On failure, remove the new suffix before exposing the target again.
    pub(crate) fn append_list_checked(
        &self,
        suffix: Value,
        check: impl FnOnce(&Value) -> Result<(), ErrorKind>,
    ) -> Result<(), ErrorKind> {
        self.with_target(|target| append_list_checked(target, suffix, check))
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

/// Invocation-local rollback of owned cells. This is not a language constant,
/// a deep copy, or a rollback of external effects. Dropping it commits changes;
/// restoration is explicit and retains every original cell identity.
pub struct OwnedGraphCheckpoint {
    entries: Vec<(Storage, Value, Responsibility)>,
}
impl fmt::Display for OwnedGraphCheckpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(owned-graph-checkpoint (cells {}))", self.entries.len())
    }
}
impl OwnedGraphCheckpoint {
    /// Acquire every cell before writing any of them. A conflicting borrow
    /// rejects restoration without partially restoring the graph. The fixed
    /// restoration work was prepaid when creating this checkpoint.
    pub fn restore(self) -> Result<(), ErrorKind> {
        let (owners, contents): (Vec<_>, Vec<_>) = self
            .entries
            .into_iter()
            .map(|(owner, value, responsibility)| (owner, (value, responsibility)))
            .unzip();
        let mut guards = Vec::with_capacity(owners.len());
        for owner in &owners {
            let responsibility = owner.1.try_lock().map_err(|_| ErrorKind::InvalidPointer)?;
            let value = owner.0.try_lock().map_err(|_| ErrorKind::InvalidPointer)?;
            guards.push((value, responsibility));
        }
        for ((value, responsibility), (saved_value, saved_responsibility)) in
            guards.iter_mut().zip(contents)
        {
            **value = saved_value;
            **responsibility = saved_responsibility;
        }
        Ok(())
    }
}

impl super::Interpreter<'_> {
    /// Read an invocation-local representation without duplicating nested
    /// storage or closure identities. This is not a language-level Copy or an
    /// exportable constant; callers must preserve the owners' lifetimes.
    pub fn read_owned_value(&mut self, storage: &Storage) -> Result<Value, super::Error> {
        let contents = storage
            .contents_for_copy(&mut self.budget.steps, self.budget.depth)
            .map_err(|kind| self.failure(kind, None))?;
        self.checkpoint_representation(
            contents.value(),
            nir::program::SourceRange { start: 0, end: 0 },
        )
    }

    /// Observe a currently readable, fixed-size scalar without copying ownership
    /// or retaining the pointer target. This is optional verifier evidence:
    /// unreadable pointers and other payloads provide no fact. Actual pointer
    /// operations still perform their ordinary lifetime/responsibility checks.
    pub fn observe_pointer_scalar(
        &mut self,
        pointer: &PointerValue,
    ) -> Result<Option<Value>, super::Error> {
        let work = pointer
            .copy_metadata_work()
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, None))?;
        self.budget.steps = self
            .budget
            .steps
            .checked_sub(work as u64)
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, None))?;
        if !pointer.is_constructed().unwrap_or(false) {
            return Ok(None);
        }
        Ok(pointer
            .inspect(|value| match value {
                Value::SignedInteger(_)
                | Value::UnsignedInteger(_)
                | Value::Float16(_)
                | Value::BFloat16(_)
                | Value::Float32(_)
                | Value::Float64(_)
                | Value::Bool(_)
                | Value::Char(_) => Some(value.clone()),
                _ => None,
            })
            .unwrap_or(None))
    }

    /// Copy a live owner's current value under the invocation budget. Pointer
    /// targets remain borrowed; copying does not retain or snapshot their owners.
    pub fn copy_owned_value(&mut self, storage: &Storage) -> Result<Value, super::Error> {
        let contents = storage
            .contents_for_copy(&mut self.budget.steps, self.budget.depth)
            .map_err(|kind| self.failure(kind, None))?;
        self.copy_representation(
            contents.value(),
            nir::program::SourceRange { start: 0, end: 0 },
            0,
        )
    }

    /// Capture cells reachable through ownership edges. Weak pointers remain
    /// leaves: callers must explicitly supply their owners among the roots.
    /// Scope and storage are local to this invocation, never serialized to IR.
    pub fn checkpoint_owned_graph(
        &mut self,
        roots: &[Value],
    ) -> Result<OwnedGraphCheckpoint, super::Error> {
        let mut cells = BTreeMap::new();
        for root in roots {
            root.visit_owned_graph(&mut self.budget.steps, &mut |value, _| {
                if let Value::Storage(storage) = value {
                    cells
                        .entry(Arc::as_ptr(&storage.0) as usize)
                        .or_insert_with(|| storage.clone());
                }
                Ok(())
            })
            .map_err(|kind| self.failure(kind, None))?;
        }
        let mut entries = Vec::new();
        for storage in cells.into_values() {
            // Reserve restoration work before storing each snapshot. Even an
            // exhausted invocation can subsequently restore its owned state.
            self.charge_preparation(2)?;
            let responsibility = storage
                .1
                .try_lock()
                .map_err(|_| self.failure(ErrorKind::InvalidPointer, None))?;
            let mut pending = vec![(&*responsibility, 0usize)];
            while let Some((node, depth)) = pending.pop() {
                self.tick(None)?;
                if depth >= 128 {
                    return Err(self.failure(ErrorKind::DepthExhausted, None));
                }
                self.charge_preparation(node.children.len())?;
                pending.extend(node.children.values().map(|child| (child, depth + 1)));
            }
            let value = storage
                .0
                .try_lock()
                .map_err(|_| self.failure(ErrorKind::InvalidPointer, None))?;
            let saved = self.checkpoint_representation(
                &value,
                nir::program::SourceRange { start: 0, end: 0 },
            )?;
            entries.push((storage.clone(), saved, responsibility.clone()));
        }
        Ok(OwnedGraphCheckpoint { entries })
    }
}

#[cfg(test)]
mod record_storage_tests {
    use super::*;
    #[test]
    fn local_reads_meter_raw_and_owned_aggregates_without_cloning_cell_identity() {
        let package = nir::program::Package::default();
        let id = nir::program::LocalId { owner: 0, index: 0 };
        let location = Some(nir::program::SourceRange { start: 7, end: 12 });
        for owned in [false, true] {
            let child = Storage::new(Value::Bool(false));
            let value = Value::Tuple(vec![Value::Storage(child.clone()), Value::Bool(true)]);
            let value = if owned {
                Value::Storage(Storage::new(value))
            } else {
                value
            };
            let environment = crate::Environment::from([(id, value)]);
            let mut vm =
                super::super::Interpreter::for_comptime(&package, Default::default()).unwrap();
            vm.budget.steps = 1;
            assert_eq!(
                vm.local_value(&environment, id, location).unwrap_err().kind,
                ErrorKind::StepsExhausted
            );
            vm.budget.steps = 100;
            let Value::Tuple(fields) = vm.local_value(&environment, id, location).unwrap() else {
                panic!()
            };
            assert_eq!(fields[0], Value::Storage(child.clone()));
            child
                .pointer(boolean(), vec![])
                .write(Value::Bool(true), |_| true)
                .unwrap();
            let Value::Storage(alias) = &fields[0] else {
                panic!()
            };
            assert_eq!(alias.read().unwrap(), Value::Bool(true));
        }
    }

    #[test]
    fn pointer_scalar_observations_are_fresh_bounded_and_non_owning() {
        let package = nir::program::Package::default();
        let mut vm = super::super::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let owner = Storage::new(Value::Bool(false));
        let pointer = owner.pointer(boolean(), vec![]);
        assert_eq!(
            vm.observe_pointer_scalar(&pointer).unwrap(),
            Some(Value::Bool(false))
        );
        pointer.write(Value::Bool(true), |_| true).unwrap();
        assert_eq!(
            vm.observe_pointer_scalar(&pointer).unwrap(),
            Some(Value::Bool(true))
        );
        vm.budget.steps = 0;
        assert_eq!(
            vm.observe_pointer_scalar(&pointer).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        vm.budget.steps = 100;
        assert_eq!(pointer.read().unwrap(), Value::Bool(true));
        pointer.transfer().unwrap();
        assert_eq!(vm.observe_pointer_scalar(&pointer).unwrap(), None);
        drop(owner);
        assert_eq!(vm.observe_pointer_scalar(&pointer).unwrap(), None);
        let aggregate = Storage::new(Value::Tuple(vec![Value::Bool(true)]));
        let aggregate_pointer = aggregate.pointer(Term::Tuple(vec![boolean()]), vec![]);
        assert_eq!(vm.observe_pointer_scalar(&aggregate_pointer).unwrap(), None);
        assert_eq!(
            aggregate.read().unwrap(),
            Value::Tuple(vec![Value::Bool(true)])
        );
    }

    #[test]
    fn owned_value_reads_preserve_shared_cells_and_bound_work() {
        let package = nir::program::Package::default();
        let mut vm = super::super::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let child = Storage::new(Value::Bool(false));
        let owner = Storage::new(Value::Tuple(vec![
            Value::Storage(child.clone()),
            Value::Storage(child.clone()),
        ]));
        let Value::Tuple(values) = vm.read_owned_value(&owner).unwrap() else {
            panic!()
        };
        assert_eq!(
            values,
            vec![Value::Storage(child.clone()), Value::Storage(child.clone())]
        );
        child
            .pointer(boolean(), vec![])
            .write(Value::Bool(true), |_| true)
            .unwrap();
        let Value::Storage(read_child) = &values[0] else {
            panic!()
        };
        assert_eq!(read_child.read().unwrap(), Value::Bool(true));
        let guard = owner.0.lock().unwrap();
        assert_eq!(
            vm.read_owned_value(&owner).unwrap_err().kind,
            ErrorKind::InvalidPointer
        );
        drop(guard);
        vm.budget.steps = 2;
        assert_eq!(
            vm.read_owned_value(&owner).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(child.read().unwrap(), Value::Bool(true));
        vm.budget.steps = 100;
        owner
            .pointer(Term::Tuple(vec![boolean(), boolean()]), vec![])
            .transfer()
            .unwrap();
        assert_eq!(
            vm.read_owned_value(&owner).unwrap_err().kind,
            ErrorKind::InvalidPointer
        );
    }

    #[test]
    fn owned_value_copy_preserves_borrows_and_checks_budget_and_responsibility() {
        let package = nir::program::Package::default();
        let mut vm = super::super::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let target = Storage::new(Value::Bool(false));
        let pointer = target.pointer(boolean(), vec![]);
        let owner = Storage::new(Value::Tuple(vec![
            Value::Bool(false),
            Value::Pointer(pointer.clone()),
        ]));
        let Value::Tuple(saved) = vm.copy_owned_value(&owner).unwrap() else {
            panic!()
        };
        *owner.0.lock().unwrap() = Value::Bool(true);
        pointer.write(Value::Bool(true), |_| true).unwrap();
        assert_eq!(saved[0], Value::Bool(false));
        let Value::Pointer(saved_pointer) = &saved[1] else {
            panic!()
        };
        assert_eq!(saved_pointer.read().unwrap(), Value::Bool(true));
        drop(target);
        assert_eq!(saved_pointer.read(), Err(ErrorKind::ExpiredPointer));

        vm.budget.steps = 0;
        assert_eq!(
            vm.copy_owned_value(&owner).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(owner.read().unwrap(), Value::Bool(true));
        vm.budget.steps = 100;
        owner.pointer(boolean(), vec![]).transfer().unwrap();
        assert_eq!(
            vm.copy_owned_value(&owner).unwrap_err().kind,
            ErrorKind::InvalidPointer
        );
    }

    #[test]
    fn owned_graph_checkpoint_restores_aliases_payload_and_responsibility() {
        let package = nir::program::Package::default();
        let mut vm = super::super::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let child = Storage::new(Value::Bool(false));
        let owner = Storage::new(Value::Tuple(vec![
            Value::Storage(child.clone()),
            Value::Storage(child.clone()),
        ]));
        let pointer = child.pointer(boolean(), vec![]);
        let ownership = owner.pointer(Term::Tuple(vec![boolean(), boolean()]), vec![]);
        let checkpoint = vm
            .checkpoint_owned_graph(&[Value::Storage(owner.clone()), Value::Storage(child.clone())])
            .unwrap();
        assert_eq!(checkpoint.to_string(), "(owned-graph-checkpoint (cells 2))");
        pointer.write(Value::Bool(true), |_| true).unwrap();
        ownership.transfer().unwrap();
        assert!(!ownership.is_constructed().unwrap());
        // Recovery does not need fresh fuel after a failed invocation.
        vm.budget.steps = 0;
        checkpoint.restore().unwrap();
        assert!(ownership.is_constructed().unwrap());
        assert_eq!(pointer.read().unwrap(), Value::Bool(false));
        let Value::Tuple(fields) = owner.read().unwrap() else {
            panic!()
        };
        assert_eq!(
            fields,
            vec![Value::Storage(child.clone()), Value::Storage(child)]
        );
    }

    #[test]
    fn owned_graph_checkpoint_does_not_follow_borrowed_owners_or_partially_restore() {
        let package = nir::program::Package::default();
        let mut vm = super::super::Interpreter::for_comptime(&package, Default::default()).unwrap();
        let foreign = Storage::new(Value::Bool(false));
        let pointer = foreign.pointer(boolean(), vec![]);
        let checkpoint = vm
            .checkpoint_owned_graph(&[Value::Pointer(pointer.clone())])
            .unwrap();
        assert_eq!(checkpoint.to_string(), "(owned-graph-checkpoint (cells 0))");
        pointer.write(Value::Bool(true), |_| true).unwrap();
        checkpoint.restore().unwrap();
        assert_eq!(foreign.read().unwrap(), Value::Bool(true));

        let a = Storage::new(Value::Bool(false));
        let b = Storage::new(Value::Bool(false));
        let checkpoint = vm
            .checkpoint_owned_graph(&[Value::Storage(a.clone()), Value::Storage(b.clone())])
            .unwrap();
        a.pointer(boolean(), vec![])
            .write(Value::Bool(true), |_| true)
            .unwrap();
        b.pointer(boolean(), vec![])
            .write(Value::Bool(true), |_| true)
            .unwrap();
        let last = checkpoint.entries.last().unwrap().0.clone();
        let guard = last.0.lock().unwrap();
        assert_eq!(checkpoint.restore(), Err(ErrorKind::InvalidPointer));
        drop(guard);
        assert_eq!(a.read().unwrap(), Value::Bool(true));
        assert_eq!(b.read().unwrap(), Value::Bool(true));
    }

    #[test]
    fn failed_checkpoint_budget_does_not_mutate_owned_data() {
        let package = nir::program::Package::default();
        let mut vm = super::super::Interpreter::for_comptime(
            &package,
            super::super::Budget {
                steps: 3,
                depth: 128,
            },
        )
        .unwrap();
        let owner = Storage::new(Value::Tuple(vec![Value::Bool(false); 32]));
        let before = owner.read().unwrap();
        assert!(matches!(
            vm.checkpoint_owned_graph(&[Value::Storage(owner.clone())]),
            Err(super::super::Error {
                kind: ErrorKind::StepsExhausted,
                ..
            })
        ));
        assert_eq!(owner.read().unwrap(), before);
    }

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

#[cfg(test)]
mod error_result_projection_tests {
    use super::*;
    #[test]
    fn cleanup_payload_pointer_keeps_result_identity_and_branch() {
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let ty = Term::error_result(vec![boolean.clone()], boolean.clone());
        let packet = |ty: Term, error| Value::ErrorResult {
            ty,
            error,
            value: Box::new(Value::Bool(true)),
        };
        let storage = Storage::new(packet(ty.clone(), None));
        let root = storage.owned_pointer(ty.clone(), &[]).unwrap();
        let payload = storage.owned_pointer(boolean.clone(), &[0]).unwrap();
        assert_eq!(payload.read().unwrap(), Value::Bool(true));
        assert!(payload.to_string().contains("(error-payload"));
        assert!(storage.owned_pointer(boolean.clone(), &[1]).is_err());
        root.write(packet(ty.clone(), Some(0)), |_| true).unwrap();
        assert_eq!(payload.read().unwrap_err(), ErrorKind::InvalidPointer);
        let failure = storage.owned_pointer(boolean.clone(), &[0]).unwrap();
        let wider = Term::error_result(
            vec![boolean.clone(), Term::Primitive(nir::Primitive::U8)],
            boolean,
        );
        root.write(packet(wider, Some(0)), |_| true).unwrap();
        assert_eq!(failure.read().unwrap_err(), ErrorKind::InvalidPointer);
    }
}

#[cfg(test)]
mod optional_projection_tests {
    use super::*;

    #[test]
    fn optional_payload_projection_checks_presence_and_full_type_identity() {
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let ty = Term::Optional(Box::new(boolean.clone()));
        let present = Value::Optional {
            ty: ty.clone(),
            value: Some(Box::new(Value::Bool(true))),
        };
        let storage = Storage::new(present);
        let root = storage.owned_pointer(ty.clone(), &[]).unwrap();
        let payload = storage.owned_pointer(boolean.clone(), &[0]).unwrap();
        assert_eq!(payload.read().unwrap(), Value::Bool(true));
        assert!(payload.to_string().contains("(optional-payload"));
        assert!(storage.owned_pointer(boolean.clone(), &[1]).is_err());
        root.write(
            Value::Optional {
                ty: ty.clone(),
                value: None,
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(payload.read().unwrap_err(), ErrorKind::InvalidPointer);
        assert!(storage.owned_pointer(boolean.clone(), &[0]).is_err());
        root.write(
            Value::Optional {
                ty: Term::Optional(Box::new(Term::Primitive(nir::Primitive::Char))),
                value: Some(Box::new(Value::Char('x'))),
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(payload.read().unwrap_err(), ErrorKind::InvalidPointer);
    }

    #[test]
    fn checkpoints_follow_optional_ownership_edges_without_copying_storage_identity() {
        let package = nir::program::Package::default();
        let mut vm = crate::Interpreter::new(&package, crate::Budget::default()).unwrap();
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let cell = Storage::new(Value::Bool(false));
        let root = Value::Optional {
            ty: Term::Optional(Box::new(boolean.clone())),
            value: Some(Box::new(Value::Storage(cell.clone()))),
        };
        let checkpoint = vm.checkpoint_owned_graph(&[root.clone()]).unwrap();
        let pointer = cell.owned_pointer(boolean, &[]).unwrap();
        pointer.write(Value::Bool(true), |_| true).unwrap();
        assert_eq!(cell.read().unwrap(), Value::Bool(true));
        checkpoint.restore().unwrap();
        assert_eq!(cell.read().unwrap(), Value::Bool(false));
        let copy = vm.read_owned_value(&Storage::new(root.clone())).unwrap();
        assert_eq!(copy, root);
        assert!(root.immutable_copy_work().is_none());
    }
}

/// Extend one list transactionally; validation cannot execute user code.
pub(super) fn append_list_checked(
    target: &mut Value,
    suffix: Value,
    check: impl FnOnce(&Value) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    let (original, summary) = match (&mut *target, suffix) {
        (
            Value::List {
                ty: left_ty,
                values: left,
            },
            Value::List {
                ty: right_ty,
                values: right,
            },
        ) if *left_ty == right_ty => {
            let original = left.len();
            if original.saturating_add(right.len()) > 100_000 {
                return Err(ErrorKind::RepresentationSizeLimit);
            }
            let summary = left.summary();
            left.append_payload(right);
            (original, summary)
        }
        _ => return Err(ErrorKind::Type),
    };
    // Validation only borrows the combined value. Keep rollback armed until
    // it returns successfully, including when a host-side check unwinds.
    struct PendingAppend<'a> {
        target: &'a mut Value,
        original: usize,
        summary: Option<super::list_values::Summary>,
        committed: bool,
    }
    impl Drop for PendingAppend<'_> {
        fn drop(&mut self) {
            if !self.committed {
                if let Value::List { values, .. } = self.target {
                    values.truncate(self.original);
                    // The check only borrowed the value; the entire prefix is
                    // unchanged, so its pre-append summary remains valid.
                    if let Some(summary) = self.summary {
                        values.remember(summary);
                    }
                }
            }
        }
    }
    let mut pending = PendingAppend {
        target,
        original,
        summary,
        committed: false,
    };
    check(pending.target)?;
    pending.committed = true;
    Ok(())
}

#[test]
fn type_list_append_restores_values_and_rejects_foreign_identity() {
    let element = Term::Primitive(nir::Primitive::Bool);
    let original = crate::test_type_list(vec![element]);
    let mut target = original.clone();
    assert_eq!(
        append_list_checked(&mut target, original.clone(), |_| Err(
            ErrorKind::StepsExhausted
        )),
        Err(ErrorKind::StepsExhausted)
    );
    assert_eq!(target, original);
    let mut foreign = original.clone();
    let Value::List {
        ty: Term::NormalForm { head, .. },
        ..
    } = &mut foreign
    else {
        panic!()
    };
    head.local += 1;
    assert_eq!(
        append_list_checked(&mut target, foreign, |_| Ok(())),
        Err(ErrorKind::Type)
    );
    assert_eq!(target, original);
    append_list_checked(&mut target, original, |_| Ok(())).unwrap();
    let Value::List { values, .. } = target else {
        panic!()
    };
    assert_eq!(values.len(), 2);
}

#[test]
fn list_append_restores_prefix_when_validation_unwinds() {
    let original = crate::test_type_list(vec![Term::Primitive(nir::Primitive::Bool)]);
    let suffix = crate::test_type_list(vec![Term::Primitive(nir::Primitive::Char)]);
    let mut target = original.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = append_list_checked(&mut target, suffix.clone(), |combined| {
            let Value::List { values, .. } = combined else {
                panic!("list expected")
            };
            assert_eq!(values.len(), 2);
            panic!("interrupted validation");
        });
    }));
    assert!(result.is_err());
    assert_eq!(target, original);
    append_list_checked(&mut target, suffix, |_| Ok(())).unwrap();
    assert_eq!(
        target,
        crate::test_type_list(vec![
            Term::Primitive(nir::Primitive::Bool),
            Term::Primitive(nir::Primitive::Char),
        ])
    );
}

#[test]
fn owned_inspection_checks_liveness_budget_and_releases_guards_after_errors() {
    let storage = Storage::new(Value::Bool(true));
    let mut exhausted = 0;
    assert_eq!(
        storage.inspect_owned(&mut exhausted, 128, |_, _| panic!("unbudgeted access")),
        Err::<(), _>(ErrorKind::StepsExhausted)
    );
    let mut fuel = 100;
    let result = storage.inspect_owned(&mut fuel, 128, |value, fuel| {
        assert_eq!(value, &Value::Bool(true));
        assert_eq!(
            storage.inspect_owned(fuel, 128, |_, _| ()),
            Err(ErrorKind::InvalidPointer)
        );
        Err::<(), _>(ErrorKind::Type)
    });
    assert_eq!(result, Ok(Err(ErrorKind::Type)));
    assert_eq!(
        storage.inspect_owned(&mut fuel, 128, |v, _| v == &Value::Bool(true)),
        Ok(true)
    );
    storage.1.lock().unwrap().constructed = false;
    assert_eq!(
        storage.inspect_owned(&mut fuel, 128, |_, _| panic!("dead storage")),
        Err::<(), _>(ErrorKind::InvalidPointer)
    );
}
