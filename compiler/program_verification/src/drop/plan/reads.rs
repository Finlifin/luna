//! Type-relative read footprints in one captured Drop selection world.
//! These retain component boundaries: no receiver storage or pointer validity
//! is assumed, and no caller-relative accesses are manufactured here.
use super::*;
use crate::{memory, moves::CleanupPolicy};

#[derive(Clone, Debug, PartialEq)]
pub struct CleanupReader {
    receiver_path: Vec<usize>,
    receiver_type: Term,
    method: MethodInstance,
    accesses: Vec<memory::Access>,
}
impl CleanupReader {
    pub fn receiver_path(&self) -> &[usize] {
        &self.receiver_path
    }
    pub fn receiver_type(&self) -> &Term {
        &self.receiver_type
    }
    pub fn method(&self) -> &MethodInstance {
        &self.method
    }
    pub fn accesses(&self) -> &[memory::Access] {
        &self.accesses
    }
}
impl fmt::Display for CleanupReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(cleanup-reader (receiver-path")?;
        for index in &self.receiver_path {
            write!(f, " {index}")?;
        }
        write!(
            f,
            ") (receiver-type {}) {} (accesses",
            self.receiver_type, self.method
        )?;
        for access in &self.accesses {
            write!(f, " {access}")?;
        }
        f.write_str("))")
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct CleanupReadFootprint {
    protocol: DeclarationId,
    subject: Term,
    readers: Vec<CleanupReader>,
}
impl CleanupReadFootprint {
    // Charge the complete owned result before cloning it into or out of the
    // session cache. A cache hit avoids proof search, not traversal limits.
    fn charge_copy(&self, fuel: &mut usize) -> Result<(), String> {
        memory::bound_type(&self.subject, fuel)?;
        for reader in &self.readers {
            *fuel = fuel
                .checked_sub(reader.receiver_path.len() + 1)
                .ok_or("cleanup footprint copy budget exhausted")?;
            memory::bound_type(&reader.receiver_type, fuel)?;
            for ty in reader
                .method
                .arguments
                .iter()
                .chain(&reader.method.method_arguments)
            {
                memory::bound_type(ty, fuel)?;
            }
            for access in &reader.accesses {
                *fuel = fuel
                    .checked_sub(access.target.path_cost())
                    .ok_or("cleanup footprint access copy budget exhausted")?;
                memory::bound_type(&access.ty, fuel)?;
            }
        }
        Ok(())
    }
    pub fn protocol(&self) -> DeclarationId {
        self.protocol
    }
    pub fn subject(&self) -> &Term {
        &self.subject
    }
    pub fn readers(&self) -> &[CleanupReader] {
        &self.readers
    }

    /// Rebase component reads onto a pointer to the complete subject.
    /// Parameter zero denotes that pointer, not an owned value argument.
    pub fn root_accesses(
        &self,
        package: &Package,
        fuel: &mut usize,
    ) -> Result<Vec<memory::Access>, String> {
        self.accesses_at(package, &self.subject, &[], fuel)
    }

    /// Place this subject at an owned path within a larger pointee storage root.
    /// Every variant condition remains attached to the load stage where it is
    /// tested. This checks layout only; callers must still establish availability,
    /// active tags, pointee validity, and the actual parameter/pointer origin.
    pub fn accesses_at(
        &self,
        package: &Package,
        root: &Term,
        receiver_path: &[usize],
        fuel: &mut usize,
    ) -> Result<Vec<memory::Access>, String> {
        let receiver = memory::project_owned_storage(Some(package), root, receiver_path, fuel)?;
        if receiver.ty != self.subject {
            return Err("cleanup footprint subject disagrees with receiver storage".into());
        }
        let mut output = vec![];
        for reader in &self.readers {
            step(fuel, 0)?;
            for access in &reader.accesses {
                step(fuel, 0)?;
                if access.write
                    || access.target.parameter != 0
                    || !access.target.input_fields.is_empty()
                    || access
                        .target
                        .conditions
                        .keys()
                        .any(|stage| *stage > access.target.through.len())
                {
                    return Err("invalid cleanup reader access boundary".into());
                }
                let mut traversal =
                    memory::StorageTraversal::new(Some(package), root, receiver_path, fuel)?;
                traversal.then(Some(package), &reader.receiver_path, fuel)?;
                if traversal.target.ty != reader.receiver_type {
                    return Err("cleanup reader path disagrees with receiver storage".into());
                }
                for (stage, fields) in access.target.through.iter().enumerate() {
                    traversal.then_guarded(
                        Some(package),
                        fields,
                        access
                            .target
                            .conditions
                            .get(&stage)
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                        fuel,
                    )?;
                    traversal.load(Some(package), &[], fuel)?;
                }
                traversal.then_guarded(
                    Some(package),
                    &access.target.fields,
                    access
                        .target
                        .conditions
                        .get(&access.target.through.len())
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    fuel,
                )?;
                if traversal.target.ty != access.ty {
                    return Err("cleanup read target disagrees with composed storage".into());
                }
                let mut through = vec![];
                let mut conditions = BTreeMap::new();
                for (stage, slot) in traversal.loads.into_iter().enumerate() {
                    step(fuel, 0)?;
                    through.push(slot.fields);
                    if !slot.guards.is_empty() {
                        conditions.insert(stage, slot.guards);
                    }
                }
                if !traversal.target.guards.is_empty() {
                    conditions.insert(through.len(), traversal.target.guards);
                }
                let rebased = memory::Access {
                    target: memory::ParameterPlace {
                        parameter: 0,
                        input_fields: vec![],
                        through,
                        fields: traversal.target.fields,
                        conditions,
                    },
                    ty: traversal.target.ty,
                    write: false,
                    read_value: access.read_value,
                };
                *fuel = fuel
                    .checked_sub(output.len().saturating_add(1))
                    .ok_or("cleanup read union budget exhausted")?;
                if !output.contains(&rebased) {
                    output.push(rebased);
                }
            }
        }
        Ok(output)
    }
}
impl fmt::Display for CleanupReadFootprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(cleanup-read-footprint (protocol {}) (subject {}) (readers",
            self.protocol, self.subject
        )?;
        for reader in &self.readers {
            write!(f, " {reader}")?;
        }
        f.write_str("))")
    }
}

fn collect(
    binding: &AcquiredCleanup,
    path: &mut Vec<usize>,
    output: &mut Vec<CleanupReader>,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, String> {
    step(fuel, depth)?;
    match &binding.resolution {
        Resolution::NoCustomDrop(_) => {}
        Resolution::Selected(selected) => {
            *fuel = fuel
                .checked_sub(path.len())
                .ok_or("cleanup receiver path budget exhausted")?;
            memory::bound_type(&binding.ty, fuel)?;
            for ty in selected
                .method
                .arguments
                .iter()
                .chain(&selected.method.method_arguments)
            {
                memory::bound_type(ty, fuel)?;
            }
            output.push(CleanupReader {
                receiver_path: path.clone(),
                receiver_type: binding.ty.clone(),
                method: selected.method.clone(),
                accesses: vec![],
            });
        }
        _ => return Ok(false),
    }
    match &binding.layout {
        Layout::Atomic => {}
        Layout::Fields(fields) => {
            for (index, field) in fields.iter().enumerate() {
                path.push(index);
                let valid = collect(field, path, output, fuel, depth + 1)?;
                path.pop();
                if !valid {
                    return Ok(false);
                }
            }
        }
        Layout::Variants(variants) => {
            for (tag, fields) in variants.iter().enumerate() {
                step(fuel, depth)?;
                for (index, field) in fields.iter().enumerate() {
                    let length = path.len();
                    path.extend([VARIANT_PATH, tag, index]);
                    let valid = collect(field, path, output, fuel, depth + 1)?;
                    path.truncate(length);
                    if !valid {
                        return Ok(false);
                    }
                }
            }
        }
    }
    Ok(true)
}
impl CleanupSession<'_, '_> {
    /// Summarize all possible owned components in this immutable selection world.
    /// Paths use the guarded owned-layout convention (including variant markers).
    /// None means the read-only, nonrejecting footprint could not be established.
    /// This neither grants Copy nor proves fields present, pointees alive, guards
    /// true, or termination. A caller must still map every reader to actual storage.
    pub fn read_footprint(
        &mut self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<CleanupReadFootprint>, String> {
        memory::bound_type(ty, fuel)?;
        if let Some(footprint) = self.read_footprints.get(ty) {
            footprint.charge_copy(fuel)?;
            return Ok(Some(footprint.clone()));
        }
        for active in &self.read_footprint_stack {
            memory::bound_type(active, fuel)?;
            if active == ty {
                return Ok(None);
            }
        }
        if self.read_footprint_stack.len() >= 64 {
            return Err("cleanup read proof nesting budget exhausted".into());
        }
        self.read_footprint_stack.push(ty.clone());
        let result = (|| {
            self.acquire(ty, fuel)?;
            let binding = self.find(ty, fuel)?.ok_or("missing acquired cleanup")?;
            let mut readers = vec![];
            if !collect(binding, &mut vec![], &mut readers, fuel, 0)? {
                return Ok(None);
            }
            let package = self.package;
            for reader in &mut readers {
                let projection =
                    memory::project_owned_storage(Some(package), ty, &reader.receiver_path, fuel)?;
                if projection.ty != reader.receiver_type {
                    return Err("cleanup reader path disagrees with acquired layout".into());
                }
                let summary = memory::infer_with_borrowed_callable_inputs_and_cleanup(
                    package,
                    reader.method.body,
                    &reader.method.arguments,
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                    &[],
                    &BTreeMap::new(),
                    Some(self),
                    fuel,
                );
                let Ok(summary) = summary else {
                    return Ok(None);
                };
                *fuel = fuel
                    .checked_sub(summary.accesses.len())
                    .ok_or("cleanup read footprint budget exhausted")?;
                if summary.may_run_cleanup
                    || summary.accesses.iter().any(|access| {
                        access.write
                            || access.target.parameter != 0
                            || !access.target.input_fields.is_empty()
                    })
                {
                    return Ok(None);
                }
                if summary.may_reject {
                    let mut rejection = crate::rejection::Cache::new(Some(package));
                    if rejection
                        .function_may_reject_with_cleanup(
                            reader.method.body,
                            &reader.method.arguments,
                            fuel,
                            &mut |ty, fuel| self.may_reject(ty, fuel),
                        )
                        .map_err(str::to_owned)?
                    {
                        return Ok(None);
                    }
                }
                reader.accesses = summary.accesses;
            }
            memory::bound_type(ty, fuel)?;
            Ok(Some(CleanupReadFootprint {
                protocol: self.protocol,
                subject: ty.clone(),
                readers,
            }))
        })();
        self.read_footprint_stack.pop();
        if let Ok(Some(footprint)) = &result {
            memory::bound_type(ty, fuel)?;
            footprint.charge_copy(fuel)?;
            self.read_footprints.insert(ty.clone(), footprint.clone());
        }
        result
    }
}
