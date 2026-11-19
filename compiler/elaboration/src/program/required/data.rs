//! Declaration data that the source owner can deliver to a waiting VM without
//! extending its callable directory or its nominal layouts.
use super::*;

use ::materialization::MemberDataSource;

#[derive(Clone, Copy)]
pub(super) struct Reader<'a> {
    pub owner: &'a MemberDataSource,
    pub form: &'a dyn Fn(
        DeclarationId,
        DeclarationId,
        &mut u64,
    ) -> Result<n::Expr, comptime_interpretor::Error>,
}

#[derive(Default)]
pub(in crate::program) struct MemberSources {
    pub(super) owner: MemberDataSource,
    data_members: BTreeSet<DeclarationId>,
}

impl MemberSources {
    /// Publish once for this checker, after complementary contributions have
    /// been merged. Every required snapshot shares this exact directory.
    pub(in crate::program) fn capture(checker: &mut Checker<'_, '_>) -> Result<Self, Error> {
        let mut slots = BTreeSet::new();
        let mut data_members = BTreeSet::new();
        let mut linked = BTreeSet::new();
        let mut spend = |count: usize| -> Result<(), Error> {
            checker.required_steps = checker
                .required_steps
                .checked_sub(count as u64)
                .ok_or_else(|| {
                    error(
                        Span::default(),
                        "associated source directory budget exhausted",
                    )
                })?;
            Ok(())
        };
        spend(checker.associated_initializers.len())?;
        for (&(row, member), recipe) in &checker.associated_initializers {
            if recipe.independent_data {
                slots.insert((row, member));
                data_members.insert(member);
            } else {
                linked.insert(member);
            }
        }
        // Complementary recipes are compared in two distinct declaration
        // scopes. Retain the exact checked payload while a contribution is active.
        spend(checker.complementary_associated.len())?;
        for &(key, _) in &checker.complementary_associated {
            slots.remove(&key);
            linked.insert(key.1);
        }
        spend(checker.checked_implementations.len())?;
        for row in &checker.checked_implementations {
            spend(
                row.associated
                    .len()
                    .saturating_add(row.associated_initializers.len()),
            )?;
            for member in row
                .associated
                .iter()
                .map(|(id, _)| id)
                .chain(row.associated_initializers.iter().map(|(id, _)| id))
            {
                if !slots.contains(&(row.declaration, *member)) {
                    linked.insert(*member);
                }
            }
        }
        for member in linked {
            data_members.remove(&member);
        }
        Ok(Self {
            owner: MemberDataSource::new(slots),
            data_members,
        })
    }

    pub(super) fn only_data(&self, member: DeclarationId) -> bool {
        self.data_members.contains(&member)
    }

    pub(super) fn supplies(&self, implementation: DeclarationId, member: DeclarationId) -> bool {
        self.owner.contains(implementation, member)
    }
}

pub(in crate::program) fn independent_member_data(
    ty: &Term,
    steps: &mut u64,
    span: Span,
) -> Result<bool, Error> {
    independent_data(ty, steps, span, 0)
}

fn independent_data(ty: &Term, steps: &mut u64, span: Span, depth: usize) -> Result<bool, Error> {
    *steps = steps
        .checked_sub(1)
        .filter(|_| depth < 128)
        .ok_or_else(|| error(span, "associated data domain observation exhausted"))?;
    match ty {
        Term::Primitive(p) => Ok(!matches!(p, P::Object | P::Type)),
        Term::Optional(inner) => independent_data(inner, steps, span, depth + 1),
        Term::Tuple(items) => fields(items, steps, span, depth),
        Term::Record(items) => fields(items.iter().map(|(_, ty)| ty), steps, span, depth),
        Term::ErrorResult { errors, result } => fields(
            errors.iter().chain(std::iter::once(result.as_ref())),
            steps,
            span,
            depth,
        ),
        _ => Ok(false),
    }
}

fn fields<'a>(
    items: impl IntoIterator<Item = &'a Term>,
    steps: &mut u64,
    span: Span,
    depth: usize,
) -> Result<bool, Error> {
    for ty in items {
        if !independent_data(ty, steps, span, depth + 1)? {
            return Ok(false);
        }
    }
    Ok(true)
}

impl Checker<'_, '_> {
    pub(super) fn required_member_data(
        &mut self,
        implementation: DeclarationId,
        member: DeclarationId,
    ) -> Result<n::Expr, Error> {
        let recipe = &self.associated_initializers[&(implementation, member)];
        let span = self
            .package
            .item(recipe.owner)
            .expect("indexed associated declaration")
            .span;
        let slot = self.ensure_associated_value(implementation, member, span)?;
        let source = interfaces::associated_source(
            &self.checked_implementations[slot],
            member,
            &mut self.required_steps,
            span,
        )?;
        let Some(nir::associated_initializers::MemberSource::Value(value)) = source else {
            return Err(error(
                span,
                "associated data owner did not publish a completed value",
            ));
        };
        constants::charge_copy(value, &mut self.required_steps, span)?;
        Ok(value.clone())
    }
}
