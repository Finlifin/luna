//! Structural evidence common to normal completions of a declaration input.
//! This fold reads checked producers. Only the original invocation runs them.
use super::*;
use dependent_inputs::tasks::spend;
use n::{NormalFormArgument as A, TypeComponent};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct Field {
    pub(super) source: Option<n::SourceRange>,
    components: VecDeque<TypeComponent>,
    constant: Option<Term>,
}

impl Field {
    pub(super) fn into_parts(self) -> (Vec<TypeComponent>, Option<Term>) {
        (self.components.into(), self.constant)
    }

    fn observed(mut term: Option<Term>, domain: &Term, source: n::SourceRange) -> Self {
        let mut components = VecDeque::new();
        if *domain == p(P::Type) {
            loop {
                let (step, inner) = match term {
                    Some(Term::Optional(inner)) => (TypeComponent::OptionalPayload, inner),
                    Some(Term::Pointer(inner)) => (TypeComponent::Pointee, inner),
                    _ => break,
                };
                components.push_back(step);
                term = Some(*inner);
            }
        }
        Self {
            source: Some(source),
            components,
            constant: term,
        }
    }

    fn intersect(&mut self, other: Self) {
        let prefix = self
            .components
            .iter()
            .zip(&other.components)
            .take_while(|(a, b)| a == b)
            .count();
        if prefix != self.components.len()
            || prefix != other.components.len()
            || self.constant != other.constant
        {
            self.constant = None;
        }
        self.components.truncate(prefix);
        if self.source != other.source {
            self.source = None;
        }
    }
}

pub(super) enum Construction {
    NoReturn,
    Field(Field),
    List(VecDeque<Field>),
}

impl Construction {
    fn unknown() -> Self {
        Self::Field(Field::default())
    }

    fn join(self, other: Self, steps: &mut u64, span: Span) -> Result<Self, Error> {
        Ok(match (self, other) {
            (Self::NoReturn, value) | (value, Self::NoReturn) => value,
            (Self::Field(mut a), Self::Field(b)) => {
                a.intersect(b);
                Self::Field(a)
            }
            (Self::List(mut a), Self::List(b)) if a.len() == b.len() => {
                spend(steps, a.len(), span)?;
                for (left, right) in a.iter_mut().zip(b) {
                    left.intersect(right);
                }
                Self::List(a)
            }
            _ => Self::unknown(),
        })
    }

    fn concat(self, other: Self, steps: &mut u64, span: Span) -> Result<Self, Error> {
        Ok(match (self, other) {
            (Self::NoReturn, _) | (_, Self::NoReturn) => Self::NoReturn,
            (Self::List(mut a), Self::List(mut b)) => {
                spend(steps, a.len().min(b.len()), span)?;
                // Move the shorter side, including in right-associated spreads.
                if b.len() > a.len() {
                    while let Some(field) = a.pop_back() {
                        b.push_front(field);
                    }
                    Self::List(b)
                } else {
                    a.append(&mut b);
                    Self::List(a)
                }
            }
            _ => Self::unknown(),
        })
    }
}

#[derive(Clone, Copy)]
struct View<'a> {
    domain: &'a Term,
    element: Option<&'a Term>,
}

impl<'a> View<'a> {
    fn scalar(domain: &'a Term) -> Self {
        Self {
            domain,
            element: None,
        }
    }
}

enum Work<'a> {
    Input(&'a A, View<'a>),
    Expr(&'a n::Expr, View<'a>),
    Collect { len: usize, inputs: &'a [A] },
    Wrap(TypeComponent),
    Join(usize),
    Concat,
}

impl Checker<'_, '_> {
    pub(super) fn observe_construction(
        &mut self,
        input: &A,
        domain: &Term,
        element: Option<&Term>,
        span: Span,
    ) -> Result<Construction, Error> {
        use Construction as C;
        use Work as W;
        let mut work = vec![W::Input(input, View { domain, element })];
        let mut values = Vec::new();
        while let Some(task) = work.pop() {
            spend(&mut self.required_steps, 1, span)?;
            match task {
                W::Join(len) => {
                    let mut common = C::NoReturn;
                    for arm in values.drain(values.len() - len..) {
                        common = common.join(arm, &mut self.required_steps, span)?;
                    }
                    values.push(common);
                }
                W::Concat => {
                    let right = values.pop().unwrap();
                    let left = values.pop().unwrap();
                    values.push(left.concat(right, &mut self.required_steps, span)?);
                }
                W::Collect { len, inputs } => {
                    let mut list = C::List(VecDeque::new());
                    for (i, child) in values.drain(values.len() - len..).enumerate() {
                        if matches!(inputs.get(i), Some(A::ListSpread(_))) {
                            list = list.concat(child, &mut self.required_steps, span)?;
                        } else {
                            match (&mut list, child) {
                                (_, C::NoReturn) => list = C::NoReturn,
                                (C::List(fields), C::Field(field)) => fields.push_back(field),
                                _ => {}
                            }
                        }
                    }
                    values.push(list);
                }
                W::Wrap(step) => {
                    if let Some(C::Field(field)) = values.last_mut() {
                        field.components.push_front(step);
                    }
                }
                W::Input(input, view) => match input {
                    A::Value(expr) | A::ListSpread(expr) => work.push(W::Expr(expr, view)),
                    A::List { items, .. } if view.element.is_some() => {
                        let elem = View::scalar(view.element.unwrap());
                        spend(&mut self.required_steps, items.len(), span)?;
                        work.push(W::Collect {
                            len: items.len(),
                            inputs: items,
                        });
                        for item in items.iter().rev() {
                            if matches!(item, A::ListSpread(_)) {
                                work.push(W::Input(item, view));
                            } else {
                                work.push(W::Input(item, elem));
                            }
                        }
                    }
                    A::If {
                        condition,
                        then_input,
                        else_input,
                        ..
                    } if !is_never(&condition.ty) => {
                        work.extend([
                            W::Join(2),
                            W::Input(else_input, view),
                            W::Input(then_input, view),
                        ]);
                    }
                    A::Match { value, arms, .. } if !is_never(&value.ty) => {
                        spend(&mut self.required_steps, arms.len(), span)?;
                        work.push(W::Join(arms.len()));
                        work.extend(arms.iter().rev().map(|a| W::Input(&a.body, view)));
                    }
                    A::Block {
                        statements, result, ..
                    } => {
                        spend(&mut self.required_steps, statements.len(), span)?;
                        if statements.iter().any(|s| is_never(&s.ty)) {
                            values.push(C::NoReturn);
                        } else if let Some(tail) = result {
                            work.push(W::Input(tail, view));
                        } else {
                            values.push(C::unknown());
                        }
                    }
                    A::If { .. } | A::Match { .. } => values.push(C::NoReturn),
                    _ => values.push(C::unknown()),
                },
                W::Expr(expr, view) => {
                    if is_never(&expr.ty) {
                        values.push(C::NoReturn);
                        continue;
                    }
                    match &expr.kind {
                        K::Block { statements, result }
                        | K::CallPreparation { statements, result } => {
                            spend(&mut self.required_steps, statements.len(), span)?;
                            if statements.iter().any(|s| is_never(&s.ty)) {
                                values.push(C::NoReturn);
                            } else if let Some(tail) = result {
                                work.push(W::Expr(tail, view));
                            } else {
                                values.push(C::unknown());
                            }
                        }
                        K::If {
                            condition,
                            then_branch,
                            else_branch,
                        } => {
                            if is_never(&condition.ty) {
                                values.push(C::NoReturn);
                            } else if let Some(other) = else_branch {
                                work.extend([
                                    W::Join(2),
                                    W::Expr(other, view),
                                    W::Expr(then_branch, view),
                                ]);
                            } else {
                                values.push(C::unknown());
                            }
                        }
                        K::Match { value, arms } => {
                            if is_never(&value.ty) {
                                values.push(C::NoReturn);
                                continue;
                            }
                            spend(&mut self.required_steps, arms.len(), span)?;
                            work.push(W::Join(arms.len()));
                            work.extend(arms.iter().rev().map(|a| W::Expr(&a.body, view)));
                        }
                        K::ListConcat(left, right) if view.element.is_some() => {
                            work.extend([W::Concat, W::Expr(right, view), W::Expr(left, view)]);
                        }
                        K::TypeList(items) if view.element.is_some() => {
                            let elem = View::scalar(view.element.unwrap());
                            spend(&mut self.required_steps, items.len(), span)?;
                            work.push(W::Collect {
                                len: items.len(),
                                inputs: &[],
                            });
                            for item in items.iter().rev() {
                                work.push(W::Expr(item, elem));
                            }
                        }
                        K::OptionalType(inner) | K::PointerType(inner)
                            if *view.domain == p(P::Type) =>
                        {
                            let step = if matches!(expr.kind, K::OptionalType(_)) {
                                TypeComponent::OptionalPayload
                            } else {
                                TypeComponent::Pointee
                            };
                            work.extend([W::Wrap(step), W::Expr(inner, view)]);
                        }
                        _ => {
                            values.push(self.constant_construction(expr, view, span)?);
                        }
                    }
                }
            }
        }
        debug_assert_eq!(values.len(), 1);
        Ok(values.pop().unwrap())
    }

    fn constant_construction(
        &mut self,
        expr: &n::Expr,
        view: View<'_>,
        span: Span,
    ) -> Result<Construction, Error> {
        let constant = self
            .annotation_constant(Some(expr), span)?
            .filter(|term| term.sort() == Sort::of_comptime_domain(view.domain));
        match (view.element, constant) {
            (Some(element), Some(term)) => {
                let fields = self.with_observation_budget(span, |_, budget| {
                    observation::optional(crate::program::normal::take_known_list_elements(
                        term,
                        view.domain,
                        element,
                        budget,
                    ))
                })?;
                Ok(fields.map_or_else(Construction::unknown, |fields| {
                    Construction::List(
                        fields
                            .into_iter()
                            .map(|term| Field::observed(Some(term), element, expr.source))
                            .collect(),
                    )
                }))
            }
            (_, constant) => Ok(Construction::Field(Field::observed(
                constant,
                view.domain,
                expr.source,
            ))),
        }
    }
}
