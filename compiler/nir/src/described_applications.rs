//! An opaque NF head applied to ordinary frozen data containing Type descriptions.
use crate::{DeclarationId, Sort, Term, program::DescriptionLiteral, type_computations};
use std::{
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

/// Each free slot of the template references one description occurrence.
/// This telescope is private to the data argument and never enters type equality.
#[derive(Debug)]
pub struct DataArgument {
    template: Term,
    fields: Vec<DescriptionLiteral>,
}

impl DataArgument {
    pub fn new(template: Term, fields: Vec<DescriptionLiteral>) -> Self {
        Self { template, fields }
    }
    pub fn concrete(term: Term) -> Self {
        Self::new(term, Vec::new())
    }
    pub fn is_concrete(&self) -> bool {
        self.fields.is_empty()
    }
    pub fn into_concrete(self) -> Option<Term> {
        self.fields.is_empty().then_some(self.template)
    }

    /// Rebase this argument's private slots into a shared validation telescope.
    pub fn in_telescope(
        &self,
        parameters: &mut Vec<crate::Parameter>,
        fuel: &mut usize,
    ) -> Result<Term, &'static str> {
        type_computations::charge_term(&self.template, fuel)
            .map_err(|_| "description input budget exhausted")?;
        *fuel = fuel
            .checked_sub(self.fields.len())
            .ok_or("description input budget exhausted")?;
        let mut slots = Vec::with_capacity(self.fields.len());
        for _ in &self.fields {
            slots.push(Term::Bound {
                depth: 0,
                index: parameters.len(),
                sort: Sort::Type,
            });
            parameters.push(crate::Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(crate::Primitive::Type),
            });
        }
        self.template
            .substitute(&slots)
            .map_err(|_| "invalid description input telescope")
    }

    fn match_input<'a>(
        &'a self,
        actual: &'a Term,
        fuel: &mut usize,
        checks: &mut Vec<(&'a DescriptionLiteral, &'a Term)>,
    ) -> Result<(), &'static str> {
        if self.fields.is_empty() {
            return if self.template == *actual {
                Ok(())
            } else {
                Err("concrete normal-form input does not match its description")
            };
        }
        let mut work = vec![(&self.template, actual)];
        let mut inputs = vec![None; self.fields.len()];
        while let Some((pattern, value)) = work.pop() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("description matching budget exhausted")?;
            if let Term::Bound {
                depth: 0,
                index,
                sort: Sort::Type,
            } = pattern
            {
                let field = self.fields.get(*index).ok_or("invalid description slot")?;
                if value.sort() != Sort::Type {
                    return Err("expected a Type in the description input");
                }
                if inputs[*index].is_some() {
                    return Err("repeated slot in frozen description input");
                }
                inputs[*index] = Some(value.clone());
                checks.push((field, value));
                continue;
            }
            // A concrete embedded type can contain function-local binders. Only
            // ordinary aggregate payloads can carry description occurrences.
            if pattern.sort() != Sort::ComptimeValue {
                continue;
            }
            let mut children = Vec::new();
            pattern.visit_children(|child| {
                children.push(child);
                Ok(())
            })?;
            let mut children = children.into_iter();
            value.visit_children(|child| {
                let next = children.next().ok_or("normal-form input shape mismatch")?;
                work.push((next, child));
                Ok(())
            })?;
            if children.len() != 0 {
                return Err("normal-form input shape mismatch");
            }
        }
        // Reconstruct the complete canonical input, including nominal identity,
        // variant/name tags and every concrete scalar. Shape alone is insufficient.
        if inputs.iter().any(Option::is_none) {
            return Err("unused slot in frozen description input");
        }
        if self
            .template
            .substitute_partial(&inputs)
            .map_err(|_| "invalid description substitution")?
            != *actual
        {
            return Err("normal-form input does not match its description");
        }
        Ok(())
    }
}

impl fmt::Display for DataArgument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(data-argument (template {}) (fields", self.template)?;
        for field in &self.fields {
            write!(f, " {field}")?;
        }
        f.write_str("))")
    }
}

#[derive(Debug)]
struct Application {
    world: Arc<()>,
    prefix: Option<crate::refinements::Refinement>,
    head: DeclarationId,
    inputs: Vec<DataArgument>,
    depth: usize,
}

#[derive(Clone, Debug)]
pub struct DescribedApplication(Arc<Application>);

impl DescribedApplication {
    pub(crate) fn new(
        world: &Arc<()>,
        head: DeclarationId,
        inputs: Vec<DataArgument>,
        registry: &crate::refinements::Refinements,
        prefix: Option<crate::refinements::Refinement>,
        fuel: &mut usize,
    ) -> Result<Self, &'static str> {
        let mut depth = 1;
        for input in &inputs {
            type_computations::charge_term(&input.template, fuel)
                .map_err(|_| "description formation budget exhausted")?;
            if type_computations::has_rigid_parameters_bounded(&input.template, fuel)
                .map_err(|_| "description formation budget exhausted")?
            {
                return Err("description application captures an outer parameter");
            }
            *fuel = fuel
                .checked_sub(input.fields.len())
                .ok_or("description formation budget exhausted")?;
            input
                .template
                .check_scope(&[vec![Sort::Type; input.fields.len()]])
                .map_err(|_| "invalid description input scope")?;
            for field in &input.fields {
                if !registry.contains(field) {
                    return Err("description input belongs to a different declaration world");
                }
                if let DescriptionLiteral::Application(inner) = field {
                    depth = depth.max(inner.0.depth + 1);
                }
            }
        }
        if depth >= 128 {
            return Err("description application nesting limit exceeded");
        }
        Ok(Self(Arc::new(Application {
            world: world.clone(),
            prefix,
            head,
            inputs,
            depth,
        })))
    }

    pub(crate) fn belongs_to(
        &self,
        world: &Arc<()>,
        registry: &crate::refinements::Refinements,
    ) -> bool {
        Arc::ptr_eq(world, &self.0.world)
            && self
                .0
                .prefix
                .as_ref()
                .is_none_or(|p| registry.contains_refinement(p))
    }
    pub fn fields_for<'a>(
        &'a self,
        actual: &'a Term,
        fuel: &mut usize,
    ) -> Result<Vec<(&'a DescriptionLiteral, &'a Term)>, &'static str> {
        type_computations::charge_term(actual, fuel)
            .map_err(|_| "description matching budget exhausted")?;
        let Term::NormalForm {
            head,
            arguments,
            sort: Sort::Type,
        } = actual
        else {
            return Err("expected the normal-form head of the parameter description");
        };
        if *head != self.0.head || arguments.len() != self.0.inputs.len() {
            return Err("normal-form head does not match the parameter description");
        }
        let mut fields = Vec::new();
        for (data, value) in self.0.inputs.iter().zip(arguments) {
            type_computations::charge_term(&data.template, fuel)
                .map_err(|_| "description matching budget exhausted")?;
            data.match_input(value, fuel, &mut fields)?;
        }
        Ok(fields)
    }
}
impl PartialEq for DescribedApplication {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for DescribedApplication {}
impl Hash for DescribedApplication {
    fn hash<H: Hasher>(&self, h: &mut H) {
        Arc::as_ptr(&self.0).hash(h);
    }
}
impl fmt::Display for DescribedApplication {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(description-application (head {}) (inputs", self.0.head)?;
        for input in &self.0.inputs {
            write!(f, " {input}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Primitive, refinements::Refinements};
    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 3, local }
    }
    fn app(head: DeclarationId, args: Vec<Term>) -> Term {
        Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: args,
        }
    }
    fn field(index: usize) -> Term {
        Term::Bound {
            depth: 0,
            index,
            sort: Sort::Type,
        }
    }

    #[test]
    fn opaque_heads_and_aggregate_labels_survive_field_matching() {
        let world = Refinements::default();
        let value = world
            .application(
                id(1),
                vec![DataArgument::new(
                    Term::RecordValue(vec![
                        ("arg".into(), field(0)),
                        ("flag".into(), Term::Bool(true)),
                    ]),
                    vec![DescriptionLiteral::Any],
                )],
                &mut 10000,
            )
            .unwrap();
        assert!(world.clone().contains(&value));
        assert!(!Refinements::default().contains(&value));
        let DescriptionLiteral::Application(desc) = value else {
            unreachable!()
        };
        let byte = Term::Primitive(Primitive::U8);
        let make = |head, name: &str| {
            app(
                head,
                vec![Term::RecordValue(vec![
                    (name.to_owned(), byte.clone()),
                    ("flag".into(), Term::Bool(true)),
                ])],
            )
        };
        let valid = make(id(1), "arg");
        assert_eq!(
            desc.fields_for(&valid, &mut 1000).unwrap(),
            vec![(&DescriptionLiteral::Any, &byte)]
        );
        assert!(desc.fields_for(&make(id(2), "arg"), &mut 1000).is_err());
        assert!(
            desc.fields_for(&make(id(1), "different"), &mut 1000)
                .is_err()
        );
        assert!(desc.fields_for(&valid, &mut 0).is_err());
        assert!(desc.fields_for(&valid, &mut 1000).is_ok());
    }
}
