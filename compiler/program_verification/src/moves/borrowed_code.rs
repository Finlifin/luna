//! Opaque call-site snapshots shared with contextual cleanup analysis.
use super::*;

pub fn copy_borrowed_code(
    inputs: &[BorrowedCode],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<Vec<BorrowedCode>, Error> {
    for input in inputs {
        *fuel = fuel
            .checked_sub(input.input_fields.len().saturating_add(1))
            .ok_or_else(|| error(source, "borrowed code copy budget exhausted"))?;
        super::super::memory::bound_type(&input.pointee, fuel).map_err(|e| error(source, e))?;
        for (path, callable) in &input.fields {
            *fuel = fuel
                .checked_sub(path.len().saturating_add(1))
                .ok_or_else(|| error(source, "borrowed code field copy budget exhausted"))?;
            bound_callable_copy(callable, fuel, source, 0)?;
        }
        for (path, tags) in &input.domains {
            *fuel = fuel
                .checked_sub(path.len().saturating_add(tags.len()).saturating_add(1))
                .ok_or_else(|| error(source, "borrowed code domain copy budget exhausted"))?;
        }
    }
    Ok(inputs.to_vec())
}

pub(super) fn borrowed_memory_inputs(
    package: Option<&Package>,
    inputs: &[BorrowedCode],
    fuel: &mut usize,
    source: SourceRange,
) -> Result<super::super::memory::BorrowedCallableInputs, Error> {
    let mut result = BTreeMap::new();
    for input in inputs {
        *fuel = fuel
            .checked_sub(input.input_fields.len().saturating_add(1))
            .ok_or_else(|| error(source, "borrowed code export budget exhausted"))?;
        let mut fields = BTreeMap::new();
        for path in callable_value_paths(package, &input.pointee, fuel, source, 0)? {
            if input.absent || enum_path_excluded(&path, &input.domains) {
                fields.insert(path, super::super::memory::BorrowedCallableValue::Absent);
            } else if let Some(callable) = input.fields.get(&path) {
                if let Some(value) = named_memory_callees(callable, fuel, source)? {
                    fields.insert(
                        path,
                        super::super::memory::BorrowedCallableValue::Named(value),
                    );
                }
            }
        }
        if !fields.is_empty() {
            let key = super::super::memory::CallableInput {
                parameter: input.parameter,
                fields: input.input_fields.clone(),
            };
            if result.insert(key, fields).is_some() {
                return Err(error(source, "duplicate borrowed memory input"));
            }
        }
    }
    Ok(result)
}

impl fmt::Display for BorrowedCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn code(value: &Callable, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "(code (definition {}) (generation {}) (alternatives",
                value.definition, value.generation
            )?;
            for alternative in &value.alternatives {
                write!(f, " {alternative}")?;
            }
            f.write_str(") (choices")?;
            for choice in &value.choices {
                f.write_str(" ")?;
                code(choice, f)?;
            }
            f.write_str(") (captures")?;
            for (place, capture) in &value.captures {
                write!(f, " (capture {place} ")?;
                code(capture, f)?;
                f.write_str(")")?;
            }
            f.write_str(") (enum-captures")?;
            for (root, domains) in &value.enum_captures {
                write!(f, " (root (owner {}) (local {})", root.owner, root.local)?;
                domain(domains, f)?;
                f.write_str(")")?;
            }
            f.write_str("))")
        }
        fn path(path: &[usize], f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("(path")?;
            for field in path {
                write!(f, " {field}")?;
            }
            f.write_str(")")
        }
        fn domain(domains: &EnumDomains, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(" (domains")?;
            for (fields, tags) in domains {
                f.write_str(" (domain ")?;
                path(fields, f)?;
                f.write_str(" (tags")?;
                for tag in tags {
                    write!(f, " {tag}")?;
                }
                f.write_str("))")?;
            }
            f.write_str(")")
        }
        write!(
            f,
            "(borrowed-code (parameter {}) (invocation {}) (absent {}) (input ",
            self.parameter, self.invocation, self.absent
        )?;
        path(&self.input_fields, f)?;
        write!(f, ") (pointee {}) (fields", self.pointee)?;
        for (fields, value) in &self.fields {
            f.write_str(" (field ")?;
            path(fields, f)?;
            f.write_str(" ")?;
            code(value, f)?;
            f.write_str(")")?;
        }
        f.write_str(")")?;
        domain(&self.domains, f)?;
        f.write_str(")")
    }
}
