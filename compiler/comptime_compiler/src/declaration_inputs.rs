//! Data needed by a declaration input activation after evaluating its callee.
use crate::{Error, copy_error};
use nir::program::{
    Expr, ExprKind as K, InputSchedule, LocalId, NormalFormArgument, NormalFormPreparation,
    SourceRange,
};
use nir::{DeclarationId, Primitive, Term};
use std::{collections::BTreeSet, fmt, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub enum Target<S, I, B> {
    Normal(DeclarationId),
    Selected {
        signature: S,
        source: SourceRange,
        input_ids: I,
    },
    Nominal(DeclarationId),
    Alias {
        declaration: DeclarationId,
        body: Option<B>,
    },
}

#[derive(Clone, Copy)]
pub struct PlanView<'a> {
    pub source: SourceRange,
    pub ty: &'a Term,
    pub target: Target<&'a Term, &'a [DeclarationId], &'a Expr>,
    pub arguments: &'a [NormalFormArgument],
    pub schedule: InputSchedule<'a>,
}

impl<'a> PlanView<'a> {
    pub fn from_expression(expr: &'a Expr) -> Option<Self> {
        let (arguments, schedule) = expr.kind.declaration_inputs()?;
        let target = match &expr.kind {
            K::NormalFormType { head, .. } => Target::Normal(*head),
            K::ApplyNormalForm {
                callee, input_ids, ..
            } => Target::Selected {
                source: callee.source,
                signature: &callee.ty,
                input_ids: input_ids.as_slice(),
            },
            K::NominalType { declaration, .. } => Target::Nominal(*declaration),
            K::CheckTypeAlias {
                declaration, body, ..
            } => Target::Alias {
                declaration: *declaration,
                body: body.as_deref(),
            },
            _ => return None,
        };
        Some(Self {
            source: expr.source,
            ty: &expr.ty,
            target,
            arguments,
            schedule,
        })
    }

    pub fn inputs_diverge(self) -> bool {
        self.arguments.iter().any(NormalFormArgument::is_diverging)
            || self
                .schedule
                .preparations
                .iter()
                .any(|p| p.expression.ty == Term::Primitive(Primitive::Never))
    }
}

/// Checked operand templates. The dynamic target's code lives exclusively in
/// the caller program. Only its interface can be observed from this plan.
#[derive(Debug)]
pub struct DeclarationInputPlan {
    source: SourceRange,
    ty: Term,
    target: Target<Term, Vec<DeclarationId>, Box<Expr>>,
    arguments: Vec<NormalFormArgument>,
    order: Option<Vec<usize>>,
    bindings: Option<Vec<LocalId>>,
    preparations: Vec<NormalFormPreparation>,
    inputs_diverge: bool,
    locals: Arc<[LocalId]>,
}

impl DeclarationInputPlan {
    pub(crate) fn compile(
        expr: &Expr,
        fuel: &mut usize,
    ) -> Result<(Self, BTreeSet<LocalId>), Error> {
        let view = PlanView::from_expression(expr).expect("declaration inputs");
        // The bounded NIR copy walk covers the result, operands, schedule and
        // alias body. It skips the shared callee. Charge its retained signature
        // separately before copying it.
        let mut steps = *fuel as u64;
        let copied = nir::copy_budget::charge_expression(expr, &mut steps);
        *fuel = steps as usize;
        copied.map_err(|e| copy_error(e, view.source))?;
        let bindings =
            nir::free_bindings::input_plan_bindings(expr, fuel).map_err(|error| match error {
                nir::free_bindings::Error::Budget(_) => Error::Budget(view.source),
                nir::free_bindings::Error::Depth(_) => Error::Depth(view.source),
                nir::free_bindings::Error::Invalid(_) => Error::Unsupported(view.source),
            })?;
        *fuel = fuel
            .checked_sub(bindings.locals.len())
            .ok_or(Error::Budget(view.source))?;
        let target = match view.target {
            Target::Normal(id) => Target::Normal(id),
            Target::Nominal(id) => Target::Nominal(id),
            Target::Selected {
                signature,
                source,
                input_ids,
            } => Target::Selected {
                signature: nir::copy_budget::clone_term(signature, fuel)
                    .map_err(|e| copy_error(e, source))?,
                source,
                input_ids: input_ids.to_vec(),
            },
            Target::Alias { declaration, body } => Target::Alias {
                declaration,
                body: body.map(|expr| Box::new(expr.clone())),
            },
        };
        let plan = Self {
            source: view.source,
            ty: view.ty.clone(),
            target,
            arguments: view.arguments.to_vec(),
            order: view.schedule.order.map(<[_]>::to_vec),
            bindings: view.schedule.bindings.map(<[_]>::to_vec),
            preparations: view.schedule.preparations.to_vec(),
            inputs_diverge: view.inputs_diverge(),
            locals: bindings.locals.into(),
        };
        Ok((plan, bindings.imports))
    }

    pub fn view(&self) -> PlanView<'_> {
        let target = match &self.target {
            Target::Normal(id) => Target::Normal(*id),
            Target::Nominal(id) => Target::Nominal(*id),
            Target::Selected {
                signature,
                source,
                input_ids,
            } => Target::Selected {
                signature,
                source: *source,
                input_ids: input_ids.as_slice(),
            },
            Target::Alias { declaration, body } => Target::Alias {
                declaration: *declaration,
                body: body.as_deref(),
            },
        };
        PlanView {
            source: self.source,
            ty: &self.ty,
            target,
            arguments: &self.arguments,
            schedule: InputSchedule {
                order: self.order.as_deref(),
                bindings: self.bindings.as_deref(),
                preparations: &self.preparations,
            },
        }
    }

    pub fn inputs_diverge(&self) -> bool {
        self.inputs_diverge
    }
    pub fn locals(&self) -> &Arc<[LocalId]> {
        &self.locals
    }
}

impl fmt::Display for PlanView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(declaration-inputs (source {} {}) (result {}) ",
            self.source.start, self.source.end, self.ty
        )?;
        match self.target {
            Target::Normal(id) => write!(f, "(normal {id})")?,
            Target::Nominal(id) => write!(f, "(nominal {id})")?,
            Target::Alias { declaration, body } => {
                write!(f, "(alias {declaration}")?;
                if let Some(body) = body {
                    write!(f, " (body {body})")?;
                }
                f.write_str(")")?;
            }
            Target::Selected {
                signature,
                source,
                input_ids,
            } => {
                write!(
                    f,
                    "(selected-interface (source {} {}) {signature} (inputs",
                    source.start, source.end
                )?;
                for id in input_ids {
                    write!(f, " {id}")?;
                }
                f.write_str("))")?;
            }
        }
        f.write_str(" (arguments")?;
        for arg in self.arguments {
            write!(f, " {arg}")?;
        }
        write!(f, ") {})", self.schedule)
    }
}

impl fmt::Display for DeclarationInputPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(declaration-input-plan (locals")?;
        for binding in self.locals.iter() {
            write!(f, " {binding}")?;
        }
        write!(
            f,
            ") (inputs-diverge {}) {})",
            self.inputs_diverge,
            self.view()
        )
    }
}
