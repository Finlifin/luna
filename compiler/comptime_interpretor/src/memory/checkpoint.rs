//! Internal rollback copies representation and ownership, never admits or
//! exports a value. VM observations can additionally validate each snapshot.
use super::*;

fn charge(fuel: &mut u64, work: usize) -> Result<(), ErrorKind> {
    *fuel = fuel
        .checked_sub(work as u64)
        .ok_or(ErrorKind::StepsExhausted)?;
    Ok(())
}

pub(super) fn capture(
    roots: &[Value],
    fuel: &mut u64,
    mut copy: impl FnMut(&Value, &mut u64) -> Result<Value, ErrorKind>,
) -> Result<OwnedGraphCheckpoint, ErrorKind> {
    let mut cells = BTreeMap::new();
    for root in roots {
        root.visit_owned_graph(fuel, &mut |value, _| {
            if let Value::Storage(storage) = value {
                cells
                    .entry(Arc::as_ptr(&storage.0) as usize)
                    .or_insert_with(|| storage.clone());
            }
            Ok(())
        })?;
    }
    let mut entries = Vec::new();
    for storage in cells.into_values() {
        charge(fuel, 2)?; // Reserve restoration before retaining an entry.
        let responsibility = storage
            .1
            .try_lock()
            .map_err(|_| ErrorKind::InvalidPointer)?;
        let mut pending = vec![(&*responsibility, 0usize)];
        while let Some((node, depth)) = pending.pop() {
            charge(fuel, 1)?;
            if depth >= 128 {
                return Err(ErrorKind::DepthExhausted);
            }
            charge(fuel, node.children.len())?;
            pending.extend(node.children.values().map(|child| (child, depth + 1)));
        }
        let value = storage
            .0
            .try_lock()
            .map_err(|_| ErrorKind::InvalidPointer)?;
        let saved = copy(&value, fuel)?;
        entries.push((storage.clone(), saved, responsibility.clone()));
    }
    Ok(OwnedGraphCheckpoint { entries })
}

impl OwnedGraphCheckpoint {
    /// Save existing storage for rollback within its owning invocation. This
    /// does not establish a source value's type, phase, Copy/Clone permission or
    /// transport authority. It retains storage/code identities, follows owned
    /// cells only, and leaves borrowed runtime pointer targets untouched.
    pub fn capture_retained(roots: &[Value], fuel: &mut u64) -> Result<Self, ErrorKind> {
        capture(roots, fuel, |value, fuel| {
            charge_clone(value, fuel, 0)?;
            Ok(value.clone())
        })
    }
}

fn term(value: &Term, fuel: &mut u64) -> Result<(), ErrorKind> {
    let before = usize::try_from(*fuel).unwrap_or(usize::MAX);
    let mut remaining = before;
    let result = nir::type_computations::charge_term(value, &mut remaining);
    *fuel -= (before - remaining) as u64;
    result.map_err(|_| ErrorKind::StepsExhausted)
}

// Exhaustive accounting for the actual derived Clone, not value interpretation.
// Immutable Arc payloads are shared. Owned cells reachable through those payloads
// are discovered separately by visit_owned_graph and receive their own entry.
fn charge_clone(value: &Value, fuel: &mut u64, depth: usize) -> Result<(), ErrorKind> {
    charge(fuel, 1)?;
    if depth >= 128 {
        return Err(ErrorKind::DepthExhausted);
    }
    match value {
        Value::Optional { ty, value } => {
            term(ty, fuel)?;
            if let Some(value) = value {
                charge_clone(value, fuel, depth + 1)?;
            }
        }
        Value::ErrorResult { ty, value, .. } | Value::Newtype { ty, value } => {
            term(ty, fuel)?;
            charge_clone(value, fuel, depth + 1)?;
        }
        Value::Type(ty) | Value::AbstractValue { ty, .. } | Value::List { ty, .. } => {
            term(ty, fuel)?
        }
        Value::Tuple(fields) => {
            charge(fuel, fields.len())?;
            for field in fields {
                charge_clone(field, fuel, depth + 1)?;
            }
        }
        Value::Record {
            arguments, fields, ..
        }
        | Value::Variant {
            arguments, fields, ..
        } => {
            charge(fuel, arguments.len().saturating_add(fields.len()))?;
            for argument in arguments {
                term(argument, fuel)?;
            }
            for field in fields {
                charge_clone(field, fuel, depth + 1)?;
            }
        }
        Value::AnonymousRecord(fields) => {
            charge(fuel, fields.len())?;
            for (name, value) in fields {
                charge(fuel, name.len())?;
                charge_clone(value, fuel, depth + 1)?;
            }
        }
        Value::Object(object) => {
            charge(fuel, object.entries().len())?;
            for entry in object.entries() {
                charge(fuel, entry.key().map_or(0, str::len))?;
                term(&entry.value().ty, fuel)?;
                charge_clone(&entry.value().value, fuel, depth + 1)?;
            }
        }
        Value::Pointer(pointer) => {
            charge(
                fuel,
                pointer
                    .copy_metadata_work()
                    .ok_or(ErrorKind::StepsExhausted)?,
            )?;
            term(&pointer.pointee, fuel)?;
        }
        Value::ComptimeReference(reference) => charge(fuel, reference.metadata_work())?,
        Value::FunctionInstance { arguments, .. } => {
            charge(fuel, arguments.len())?;
            for argument in arguments {
                term(argument, fuel)?;
            }
        }
        Value::RuntimeTraitMethod(method) => {
            term(&method.subject, fuel)?;
            charge(
                fuel,
                method
                    .interface
                    .arguments
                    .len()
                    .saturating_add(method.method_arguments.len()),
            )?;
            for argument in method
                .interface
                .arguments
                .iter()
                .chain(&method.method_arguments)
            {
                term(argument, fuel)?;
            }
        }
        Value::Integer(value) => charge(fuel, value.bit_len().div_ceil(32))?,
        Value::Float(value) => {
            charge(fuel, value.digits().len())?;
            charge(fuel, value.exponent().bit_len().div_ceil(32))?;
        }
        Value::BitVec(value) => charge(fuel, value.words().len())?,
        Value::Code(_)
        | Value::Storage(_)
        | Value::Closure(_)
        | Value::String(_)
        | Value::Symbol(_)
        | Value::Description(_)
        | Value::TypeParameter(_)
        | Value::Float16(_)
        | Value::BFloat16(_)
        | Value::Float32(_)
        | Value::Float64(_)
        | Value::SignedInteger(_)
        | Value::UnsignedInteger(_)
        | Value::Unit
        | Value::Bool(_)
        | Value::Char(_)
        | Value::Function(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_checkpoint_preserves_aliases_moves_and_borrowed_targets() {
        let child = Storage::new(Value::Bool(false));
        let borrowed = Storage::new(Value::Bool(false));
        let pointer = borrowed.pointer(Term::Primitive(nir::Primitive::Bool), vec![]);
        let root = Storage::new(Value::Tuple(vec![
            Value::Storage(child.clone()),
            Value::Storage(child.clone()),
            Value::Pointer(pointer),
        ]));
        root.1.lock().unwrap().position(&[1]).constructed = false;
        let roots = [Value::Storage(root.clone()), Value::Storage(child.clone())];
        let checkpoint = OwnedGraphCheckpoint::capture_retained(&roots, &mut 10000).unwrap();
        assert_eq!(checkpoint.entries.len(), 2);
        *root.0.lock().unwrap() = Value::Unit;
        root.1.lock().unwrap().children.clear();
        *child.0.lock().unwrap() = Value::Bool(true);
        *borrowed.0.lock().unwrap() = Value::Bool(true);
        checkpoint.restore().unwrap();
        let Value::Tuple(fields) = root.read().unwrap() else {
            panic!()
        };
        assert_eq!(fields[0], Value::Storage(child.clone()));
        assert_eq!(fields[1], fields[0]);
        assert_eq!(child.read().unwrap(), Value::Bool(false));
        assert!(!root.1.lock().unwrap().children[&1].constructed);
        assert_eq!(borrowed.read().unwrap(), Value::Bool(true));
    }

    #[test]
    fn interrupted_capture_is_read_only_and_does_not_admit_descriptions() {
        let ty = Term::Nominal {
            declaration: DeclarationId {
                package: 400,
                local: 7,
            },
            arguments: vec![],
        };
        let root = Storage::new(Value::Object(
            comptime_runtime::object::Object::from_entries(vec![
                comptime_runtime::object::Entry::Value(crate::ObjectValue {
                    ty,
                    value: Value::Bool(false),
                }),
            ]),
        ));
        let roots = [Value::Storage(root.clone())];
        let mut fuel = 10000;
        let original = root.read().unwrap();
        let checkpoint = OwnedGraphCheckpoint::capture_retained(&roots, &mut fuel).unwrap();
        let work = 10000 - fuel;
        for allowance in 0..work {
            assert!(matches!(
                OwnedGraphCheckpoint::capture_retained(&roots, &mut { allowance }),
                Err(ErrorKind::StepsExhausted)
            ));
            assert_eq!(root.read().unwrap(), original);
        }
        let package = nir::program::Package::default();
        let mut vm = crate::Interpreter::for_comptime(&package, Default::default()).unwrap();
        assert!(vm.checkpoint_owned_graph(&roots).is_err());
        *root.0.lock().unwrap() = Value::Unit;
        checkpoint.restore().unwrap();
        assert_eq!(root.read().unwrap(), original);
        let mut nested = Value::Bool(false);
        for _ in 0..130 {
            nested = Value::Tuple(vec![nested]);
        }
        assert!(matches!(
            OwnedGraphCheckpoint::capture_retained(
                &[Value::Storage(Storage::new(nested))],
                &mut 10000
            ),
            Err(ErrorKind::DepthExhausted)
        ));
    }
}
