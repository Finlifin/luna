//! Type-parameter inference and ordered runtime-dependent call substitution.
//! The caller supplies stage-authorized signatures and already-routed inputs;
//! this module neither evaluates arguments nor invents runtime object identity.
use crate::{
    InferenceTerm as I,
    matching::MatchError,
    obligations::{Engine, Error as ObligationError},
    ptc::Budget,
};
use nir::{DeclarationId, Sort, Term, TypeError};

mod domain_staging;

pub struct Scheme {
    /// Empty preserves Type-only callers; otherwise one sort per clause input.
    pub parameter_sorts: Vec<Sort>,
    /// One surrounding frame of clause parameters; fn introduces its own frame.
    pub type_parameters: usize,
    pub signature: Term,
}
pub struct Argument {
    pub value: Term,
    pub ty: Term,
}
/// Ephemeral checking context; unknown components never become NIR types.
#[derive(Clone, Debug)]
pub struct CallableContext {
    pub comptime: bool,
    pub environment: Option<Vec<Option<Term>>>,
    /// Environment extent is unresolved; None alone denotes an ordinary
    /// environment-free callable. This is candidate-local information only.
    pub open_environment: bool,
    pub parameters: Vec<Option<Term>>,
    pub result: Option<Term>,
    /// Used only while the result is incomplete; shares the candidate context budget.
    pub result_context: Option<Box<InputContext>>,
}
/// Partial structural input checking context. Unresolved inference terms remain private
/// to the candidate and are never published as NIR types.
#[derive(Clone, Debug)]
pub enum InputContext {
    Optional(Box<InputContext>),
    ErrorResult {
        errors: Vec<Term>,
        result: Box<InputContext>,
    },
    Computation {
        effects: Vec<nir::effects::Operation>,
        result: Box<InputContext>,
    },
    Known(Term),
    Tuple(Vec<InputContext>),
    NormalTuple {
        head: DeclarationId,
        fields: Vec<InputContext>,
    },
    /// A generative nominal type with partially known Type and value arguments.
    Nominal {
        declaration: DeclarationId,
        arguments: Vec<InputContext>,
    },
    Record(Vec<(String, InputContext)>),
    Callable(CallableContext),
    Unknown,
    /// An unresolved comptime value index, not an unresolved Type.
    UnknownValue,
    /// An unresolved value whose domain is itself an unresolved clause input.
    /// Its sort may become Type when the earlier domain input is completed.
    UnknownDependentValue,
}
pub struct CallCandidate {
    parameter_sorts: Vec<Sort>,
    dependent_value_parameters: Vec<usize>,
    engine: Engine,
    types: Vec<I>,
    signature: I,
    result: I,
    // The ordinary result or the delivered result after one effect layer.
    // None retains the existing wait for unresolved computation inputs.
    contextual_result: Option<I>,
    views: std::collections::HashMap<Term, Term>,
}
#[derive(Debug)]
pub enum Error {
    NeedsContext,
    InvalidSignature,
    ArgumentCount,
    TypeArgumentCount,
    Shape(TypeError),
    Inference(crate::InferenceError),
    Match(MatchError),
    Obligations(ObligationError),
    UnsettledRequirement {
        cause: ObligationError,
        requirement: String,
        status: &'static str,
    },
    TypeView(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsettledRequirement {
                requirement,
                status,
                ..
            } => write!(f, "required type constraint {requirement}: {status}"),
            other => write!(f, "{other:?}"),
        }
    }
}
impl From<TypeError> for Error {
    fn from(e: TypeError) -> Self {
        Self::Shape(e)
    }
}
pub struct Instance {
    pub type_arguments: Vec<Term>,
    pub signature: Term,
    pub result: Term,
}
impl Scheme {
    fn input_sorts(&self) -> Result<Vec<Sort>, Error> {
        if self.parameter_sorts.is_empty() {
            return Ok(vec![Sort::Type; self.type_parameters]);
        }
        if self.parameter_sorts.len() != self.type_parameters
            || self
                .parameter_sorts
                .iter()
                .any(|sort| !matches!(sort, Sort::Type | Sort::ComptimeValue))
        {
            return Err(Error::InvalidSignature);
        }
        Ok(self.parameter_sorts.clone())
    }
    /// Type-only call inference when the signature has no runtime dependencies.
    /// Removing the runtime frame with no arguments deliberately rejects such
    /// dependencies instead of inventing runtime object identities.
    pub fn infer_types(
        &self,
        actual: &[Term],
        expected: Option<&Term>,
        budget: Budget,
    ) -> Result<CallCandidate, Error> {
        self.infer_partial_types(
            &actual.iter().cloned().map(Some).collect::<Vec<_>>(),
            expected,
            budget,
        )
    }
    pub fn infer_partial_types(
        &self,
        actual: &[Option<Term>],
        expected: Option<&Term>,
        budget: Budget,
    ) -> Result<CallCandidate, Error> {
        self.infer_partial_types_in_context(
            actual,
            expected,
            &crate::ptc::Context::default(),
            budget,
        )
    }
    pub fn infer_partial_types_in_context(
        &self,
        actual: &[Option<Term>],
        expected: Option<&Term>,
        context: &crate::ptc::Context,
        budget: Budget,
    ) -> Result<CallCandidate, Error> {
        self.infer_contextual_types(actual, expected, None, context, budget)
    }
    pub fn infer_contextual_types(
        &self,
        actual: &[Option<Term>],
        expected: Option<&Term>,
        partial: Option<&CallableContext>,
        context: &crate::ptc::Context,
        budget: Budget,
    ) -> Result<CallCandidate, Error> {
        self.infer_contextual_types_mode(actual, expected, partial, context, budget, false)
    }
    pub(crate) fn infer_contextual_types_mode(
        &self,
        actual: &[Option<Term>],
        expected: Option<&Term>,
        partial: Option<&CallableContext>,
        context: &crate::ptc::Context,
        budget: Budget,
        discharge_effects: bool,
    ) -> Result<CallCandidate, Error> {
        self.infer_scoped_types_mode(
            actual,
            expected,
            partial,
            context,
            budget,
            discharge_effects,
            &[],
        )
    }
    /// Infer inside an explicitly supplied caller telescope. Returned terms
    /// remain relative to that scope; the callee's own binders cannot escape.
    pub fn infer_scoped_types(
        &self,
        actual: &[Option<Term>],
        expected: Option<&Term>,
        context: &crate::ptc::Context,
        budget: Budget,
        scope: &[nir::Parameter],
    ) -> Result<CallCandidate, Error> {
        self.infer_scoped_types_mode(actual, expected, None, context, budget, false, scope)
    }
    pub(crate) fn infer_scoped_types_mode(
        &self,
        actual: &[Option<Term>],
        expected: Option<&Term>,
        partial: Option<&CallableContext>,
        context: &crate::ptc::Context,
        budget: Budget,
        discharge_effects: bool,
        scope: &[nir::Parameter],
    ) -> Result<CallCandidate, Error> {
        Term::Function {
            parameters: scope.to_vec(),
            result: Box::new(Term::Primitive(nir::Primitive::Void)),
        }
        .check_scope(&[])?;
        let caller_frames = if scope.is_empty() {
            vec![]
        } else {
            vec![scope.iter().map(|p| p.sort).collect()]
        };
        self.infer_in_caller(
            actual,
            expected,
            partial,
            budget,
            discharge_effects,
            caller_frames,
            Engine::with_call_scope(context.fork(), scope.to_vec()),
        )
    }

    /// Infer under a caller telescope whose runtime domains may still be open.
    /// Missing domains are not types and never become inference substitutions.
    pub fn infer_sparse_scoped_types(
        &self,
        actual: &[Option<Term>],
        expected: Option<&Term>,
        context: &crate::ptc::Context,
        budget: Budget,
        scope: &[(Sort, Option<Term>)],
    ) -> Result<CallCandidate, Error> {
        self.infer_sparse_scoped_types_mode(actual, expected, None, context, budget, false, scope)
    }

    pub fn infer_sparse_scoped_types_mode(
        &self,
        actual: &[Option<Term>],
        expected: Option<&Term>,
        partial: Option<&CallableContext>,
        context: &crate::ptc::Context,
        budget: Budget,
        discharge_effects: bool,
        scope: &[(Sort, Option<Term>)],
    ) -> Result<CallCandidate, Error> {
        if let Some(parameters) = scope
            .iter()
            .map(|(sort, domain)| domain.clone().map(|ty| nir::Parameter { sort: *sort, ty }))
            .collect::<Option<Vec<_>>>()
        {
            return self.infer_scoped_types_mode(
                actual,
                expected,
                partial,
                context,
                budget,
                discharge_effects,
                &parameters,
            );
        }
        let caller_frames = if scope.is_empty() {
            vec![]
        } else {
            vec![scope.iter().map(|entry| entry.0).collect()]
        };
        let root = Engine::with_sparse_call_scope(context.fork(), scope.to_vec())
            .map_err(Error::Obligations)?;
        self.infer_in_caller(
            actual,
            expected,
            partial,
            budget,
            discharge_effects,
            caller_frames,
            root,
        )
    }

    fn infer_in_caller(
        &self,
        actual: &[Option<Term>],
        expected: Option<&Term>,
        partial: Option<&CallableContext>,
        budget: Budget,
        discharge_effects: bool,
        caller_frames: Vec<Vec<Sort>>,
        mut root: Engine,
    ) -> Result<CallCandidate, Error> {
        let Term::Function { parameters, result } = &self.signature else {
            return Err(Error::InvalidSignature);
        };
        if parameters.len() != actual.len() {
            return Err(Error::ArgumentCount);
        }
        let mut frames = caller_frames.clone();
        let parameter_sorts = self.input_sorts()?;
        frames.push(parameter_sorts.clone());
        self.signature.check_scope(&frames)?;
        let types: Vec<_> = parameter_sorts
            .iter()
            .map(|sort| I::Meta(root.fresh(*sort)))
            .collect();
        let mut relations = Vec::new();
        for (param, actual) in parameters.iter().zip(actual) {
            // Argument type inference is independent of delivery stage. The
            // expression checker supplies required invocation for staged slots.
            let template = param.ty.substitute(&[])?;
            let Some(actual) = actual else {
                continue;
            };
            if actual.sort() != Sort::Type {
                return Err(Error::InvalidSignature);
            }
            actual.check_scope(&caller_frames)?;
            relations.push((
                I::Instantiate {
                    template,
                    arguments: types.clone(),
                },
                I::Stable(actual.clone()),
            ));
        }
        let result = I::Instantiate {
            template: result.substitute(&[])?,
            arguments: types.clone(),
        };
        let defer_effect_context = discharge_effects
            && parameters.iter().zip(actual).any(|(parameter, actual)| {
                actual.is_none() && matches!(parameter.ty, Term::Computation { .. })
            });
        let contextual_result = if discharge_effects {
            let (preview, _) = root.match_many(&relations, budget).map_err(Error::Match)?;
            if let Ok(I::Stable(ty)) = preview.inspect(&result) {
                I::Stable(if let Term::Computation { result, .. } = ty {
                    *result
                } else {
                    ty
                })
            } else {
                let I::Instantiate {
                    template,
                    arguments,
                } = &result
                else {
                    unreachable!()
                };
                if let Term::Computation { result, .. } = template {
                    I::Instantiate {
                        template: result.as_ref().clone(),
                        arguments: arguments.clone(),
                    }
                } else {
                    result.clone()
                }
            }
        } else {
            result.clone()
        };
        if let Some(expected) = expected.filter(|_| !defer_effect_context) {
            expected.check_scope(&caller_frames)?;
            relations.push((contextual_result.clone(), I::Stable(expected.clone())));
        }
        if let Some(partial) = partial.filter(|_| !defer_effect_context) {
            if partial.open_environment && partial.environment.is_some() {
                return Err(Error::InvalidSignature);
            }
            let mut components = Vec::new();
            for known in partial
                .parameters
                .iter()
                .chain(std::iter::once(&partial.result))
                .chain(partial.environment.iter().flatten())
            {
                components.push(match known {
                    Some(ty) => {
                        ty.check_scope(&caller_frames)?;
                        if ty.sort() != Sort::Type {
                            return Err(Error::InvalidSignature);
                        }
                        I::Stable(ty.clone())
                    }
                    None => I::Meta(root.fresh(Sort::Type)),
                });
            }
            let slot = |index| Term::Bound {
                depth: 1,
                index,
                sort: Sort::Type,
            };
            let template = Term::Function {
                parameters: (0..partial.parameters.len())
                    .map(|index| nir::Parameter {
                        sort: Sort::Runtime,
                        ty: slot(index),
                    })
                    .collect(),
                result: Box::new(slot(partial.parameters.len())),
            }
            .with_comptime_phase(partial.comptime);
            let template = if partial.open_environment {
                let index = components.len();
                components.push(I::Meta(root.fresh(Sort::ComptimeValue)));
                Term::contextual_closure(
                    Term::Bound {
                        depth: 0,
                        index,
                        sort: Sort::ComptimeValue,
                    },
                    template,
                )?
            } else if let Some(environment) = &partial.environment {
                let fields = (0..environment.len())
                    .map(|index| Term::Bound {
                        depth: 0,
                        index: partial.parameters.len() + 1 + index,
                        sort: Sort::Type,
                    })
                    .collect();
                Term::contextual_closure(Term::ClosureContextValue(fields), template)?
            } else {
                template
            };
            relations.push((
                contextual_result.clone(),
                I::Instantiate {
                    template,
                    arguments: components,
                },
            ));
        }
        let (mut engine, _) = root.match_many(&relations, budget).map_err(Error::Match)?;
        engine.advance(relations.len(), budget);
        let mut candidate = CallCandidate {
            parameter_sorts,
            dependent_value_parameters: vec![],
            views: Default::default(),
            engine,
            types: types.clone(),
            contextual_result: (!defer_effect_context).then_some(contextual_result),
            result,
            signature: I::Instantiate {
                template: self.signature.clone(),
                arguments: types,
            },
        };
        if let Some(partial) =
            partial.filter(|context| context.result_context.is_some() && !defer_effect_context)
        {
            candidate.constrain_result_context(&InputContext::Callable(partial.clone()), budget)?;
        }
        Ok(candidate)
    }
    pub fn infer(
        &self,
        arguments: &[Argument],
        explicit: &[Option<Term>],
        expected: Option<&Term>,
        budget: Budget,
    ) -> Result<CallCandidate, Error> {
        let Term::Function { parameters, result } = &self.signature else {
            return Err(Error::InvalidSignature);
        };
        if parameters.iter().any(|p| p.sort != Sort::Runtime) {
            return Err(Error::InvalidSignature);
        }
        let parameter_sorts = self.input_sorts()?;
        self.signature.check_scope(&[parameter_sorts.clone()])?;
        if arguments.len() != parameters.len() {
            return Err(Error::ArgumentCount);
        }
        if explicit.len() != self.type_parameters {
            return Err(Error::TypeArgumentCount);
        }
        let mut root = Engine::default();
        let mut types = Vec::new();
        for (supplied, sort) in explicit.iter().zip(&parameter_sorts) {
            let meta = root.fresh(*sort);
            if let Some(ty) = supplied {
                ty.check_scope(&[])?;
                root.assign(meta, I::Stable(ty.clone()))
                    .map_err(Error::Obligations)?;
            }
            types.push(I::Meta(meta));
        }
        let mut values = Vec::new();
        let mut relations = Vec::new();
        for (parameter, argument) in parameters.iter().zip(arguments) {
            if argument.value.sort() != Sort::Runtime || argument.ty.sort() != Sort::Type {
                return Err(Error::InvalidSignature);
            }
            argument.value.check_scope(&[])?;
            argument.ty.check_scope(&[])?;
            // Remove only the runtime call frame; the surrounding type frame
            // remains for inference. Earlier input objects remain distinct.
            let template = parameter.ty.substitute(&values)?;
            relations.push((
                I::Instantiate {
                    template,
                    arguments: types.clone(),
                },
                I::Stable(argument.ty.clone()),
            ));
            values.push(argument.value.clone());
        }
        let result = I::Instantiate {
            template: result.substitute(&values)?,
            arguments: types.clone(),
        };
        if let Some(expected) = expected {
            expected.check_scope(&[])?;
            relations.push((result.clone(), I::Stable(expected.clone())));
        }
        let (mut engine, _) = root.match_many(&relations, budget).map_err(Error::Match)?;
        engine.advance(relations.len(), budget);
        Ok(CallCandidate {
            dependent_value_parameters: vec![],
            parameter_sorts,
            views: Default::default(),
            engine,
            types: types.clone(),
            signature: I::Instantiate {
                template: self.signature.clone(),
                arguments: types,
            },
            contextual_result: Some(result.clone()),
            result,
        })
    }
}
impl CallCandidate {
    /// Import only structural checking information. Unknown components receive
    /// fresh variables in this candidate, never variables owned by its caller.
    pub fn constrain_result_context(
        &mut self,
        context: &InputContext,
        budget: Budget,
    ) -> Result<(), Error> {
        let Some(target) = self.contextual_result.clone() else {
            return Ok(());
        };
        self.constrain_component_context(target, context, budget)
    }

    pub fn constrain_parameter_context(
        &mut self,
        index: usize,
        context: &InputContext,
        budget: Budget,
    ) -> Result<(), Error> {
        self.constrain_component_context(self.parameter_term(index)?, context, budget)
    }

    fn constrain_component_context(
        &mut self,
        target: I,
        context: &InputContext,
        budget: Budget,
    ) -> Result<(), Error> {
        fn leaf(
            ty: Option<&Term>,
            engine: &mut Engine,
            components: &mut Vec<I>,
            nesting: u32,
            fuel: &mut usize,
        ) -> Result<Term, Error> {
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| Error::TypeView("result context budget exhausted".into()))?;
            let index = components.len();
            components.push(if let Some(ty) = ty {
                if ty.sort() != Sort::Type {
                    return Err(Error::InvalidSignature);
                }
                crate::associated::charge_projection_copy(ty, fuel).map_err(Error::TypeView)?;
                engine.check_context_type(ty).map_err(Error::Obligations)?;
                I::Stable(ty.clone())
            } else {
                I::Meta(engine.fresh(Sort::Type))
            });
            Ok(Term::Bound {
                depth: nesting,
                index,
                sort: Sort::Type,
            })
        }
        fn build(
            context: &InputContext,
            settled: Option<&Term>,
            engine: &mut Engine,
            components: &mut Vec<I>,
            nesting: u32,
            depth: usize,
            fuel: &mut usize,
        ) -> Result<Term, Error> {
            if depth >= 128 || *fuel == 0 {
                return Err(Error::TypeView("result context budget exhausted".into()));
            }
            *fuel -= 1;
            match context {
                InputContext::Optional(payload) => Ok(Term::Optional(Box::new(build(
                    payload,
                    match settled {
                        Some(Term::Optional(inner)) => Some(inner.as_ref()),
                        _ => None,
                    },
                    engine,
                    components,
                    nesting,
                    depth + 1,
                    fuel,
                )?))),
                InputContext::ErrorResult { errors, result } => {
                    for label in errors {
                        crate::associated::charge_projection_copy(label, fuel)
                            .map_err(Error::TypeView)?;
                        engine
                            .check_context_type(label)
                            .map_err(Error::Obligations)?;
                    }
                    Ok(Term::error_result(
                        errors.clone(),
                        build(
                            result,
                            match settled {
                                Some(Term::ErrorResult { result, .. }) => Some(result.as_ref()),
                                _ => None,
                            },
                            engine,
                            components,
                            nesting,
                            depth + 1,
                            fuel,
                        )?,
                    ))
                }
                InputContext::Computation { effects, result } => {
                    let row =
                        Term::computation(effects.clone(), Term::Primitive(nir::Primitive::Void));
                    crate::associated::charge_projection_copy(&row, fuel)
                        .map_err(Error::TypeView)?;
                    engine
                        .check_context_type(&row)
                        .map_err(Error::Obligations)?;
                    Ok(Term::computation(
                        effects.clone(),
                        build(
                            result,
                            match settled {
                                Some(Term::Computation { result, .. }) => Some(result.as_ref()),
                                _ => None,
                            },
                            engine,
                            components,
                            nesting,
                            depth + 1,
                            fuel,
                        )?,
                    ))
                }
                InputContext::Known(ty) => leaf(Some(ty), engine, components, nesting, fuel),
                InputContext::Unknown => leaf(None, engine, components, nesting, fuel),
                InputContext::UnknownDependentValue
                    if settled.is_some_and(|term| term.sort() == Sort::Type) =>
                {
                    // A dependent value domain may have completed to Type since
                    // the caller exported this partial context. Unknown carries
                    // no value evidence; use the candidate's completed sort.
                    leaf(None, engine, components, nesting, fuel)
                }
                InputContext::UnknownValue | InputContext::UnknownDependentValue => {
                    let index = components.len();
                    components.push(I::Meta(engine.fresh(Sort::ComptimeValue)));
                    Ok(Term::Bound {
                        depth: nesting,
                        index,
                        sort: Sort::ComptimeValue,
                    })
                }
                InputContext::Nominal {
                    declaration,
                    arguments,
                } => {
                    if !engine.is_nominal_constructor(*declaration) {
                        return Err(Error::TypeView(
                            "unregistered nominal result context".into(),
                        ));
                    }
                    Ok(Term::Nominal {
                        declaration: *declaration,
                        arguments: arguments
                            .iter()
                            .enumerate()
                            .map(|(slot, argument)| {
                                let settled = match settled {
                                    Some(Term::Nominal {
                                        declaration: head,
                                        arguments,
                                    }) if head == declaration => arguments.get(slot),
                                    _ => None,
                                };
                                if let InputContext::Known(value) = argument {
                                    if value.sort() == Sort::ComptimeValue {
                                        crate::associated::charge_projection_copy(value, fuel)
                                            .map_err(Error::TypeView)?;
                                        engine
                                            .check_context_type(value)
                                            .map_err(Error::Obligations)?;
                                        let index = components.len();
                                        components.push(I::Stable(value.clone()));
                                        return Ok(Term::Bound {
                                            depth: nesting,
                                            index,
                                            sort: Sort::ComptimeValue,
                                        });
                                    }
                                }
                                build(
                                    argument,
                                    settled,
                                    engine,
                                    components,
                                    nesting,
                                    depth + 1,
                                    fuel,
                                )
                            })
                            .collect::<Result<_, _>>()?,
                    })
                }
                InputContext::Tuple(fields) => Ok(Term::Tuple(
                    fields
                        .iter()
                        .enumerate()
                        .map(|(slot, field)| {
                            build(
                                field,
                                match settled {
                                    Some(Term::Tuple(fields)) => fields.get(slot),
                                    _ => None,
                                },
                                engine,
                                components,
                                nesting,
                                depth + 1,
                                fuel,
                            )
                        })
                        .collect::<Result<_, _>>()?,
                )),
                InputContext::NormalTuple { head, fields } => {
                    if !engine.is_tuple_head(*head) {
                        return Err(Error::TypeView("unbound Tuple result context".into()));
                    }
                    Ok(Term::NormalForm {
                        head: *head,
                        sort: Sort::Type,
                        arguments: vec![Term::TypeListValue(
                            fields
                                .iter()
                                .enumerate()
                                .map(|(slot, field)| {
                                    let field_hint = match settled {
                                        Some(Term::NormalForm {
                                            head: actual,
                                            arguments,
                                            ..
                                        }) if actual == head => match arguments.as_slice() {
                                            [Term::TypeListValue(fields)] => fields.get(slot),
                                            _ => None,
                                        },
                                        _ => None,
                                    };
                                    build(
                                        field,
                                        field_hint,
                                        engine,
                                        components,
                                        nesting,
                                        depth + 1,
                                        fuel,
                                    )
                                })
                                .collect::<Result<_, _>>()?,
                        )],
                    })
                }
                InputContext::Record(fields) => Ok(Term::Record(
                    fields
                        .iter()
                        .enumerate()
                        .map(|(slot, (name, field))| {
                            let field_hint = match settled {
                                Some(Term::Record(fields)) => fields
                                    .get(slot)
                                    .filter(|(actual, _)| actual == name)
                                    .map(|(_, ty)| ty),
                                _ => None,
                            };
                            build(
                                field,
                                field_hint,
                                engine,
                                components,
                                nesting,
                                depth + 1,
                                fuel,
                            )
                            .map(|ty| (name.clone(), ty))
                        })
                        .collect::<Result<_, _>>()?,
                )),
                InputContext::Callable(callable) => {
                    if callable.open_environment && callable.environment.is_some() {
                        return Err(Error::InvalidSignature);
                    }
                    let mut component = |ty: &Option<Term>, nesting| {
                        leaf(ty.as_ref(), engine, components, nesting, fuel)
                    };
                    let parameters = callable
                        .parameters
                        .iter()
                        .map(|ty| {
                            Ok(nir::Parameter {
                                sort: Sort::Runtime,
                                ty: component(ty, nesting + 1)?,
                            })
                        })
                        .collect::<Result<_, Error>>()?;
                    let result = if let Some(result) = &callable.result {
                        leaf(Some(result), engine, components, nesting + 1, fuel)?
                    } else if let Some(context) = &callable.result_context {
                        build(
                            context,
                            settled.and_then(|ty| match ty.callable_signature() {
                                Term::Function { result, .. } => Some(result.as_ref()),
                                _ => None,
                            }),
                            engine,
                            components,
                            nesting + 1,
                            depth + 1,
                            fuel,
                        )?
                    } else {
                        leaf(None, engine, components, nesting + 1, fuel)?
                    };
                    let signature = Term::Function {
                        parameters,
                        result: Box::new(result),
                    }
                    .with_comptime_phase(callable.comptime);
                    Ok(if callable.open_environment {
                        let index = components.len();
                        components.push(I::Meta(engine.fresh(Sort::ComptimeValue)));
                        Term::contextual_closure(
                            Term::Bound {
                                depth: nesting,
                                index,
                                sort: Sort::ComptimeValue,
                            },
                            signature,
                        )?
                    } else if let Some(environment) = &callable.environment {
                        let fields = environment
                            .iter()
                            .map(|ty| leaf(ty.as_ref(), engine, components, nesting, fuel))
                            .collect::<Result<_, _>>()?;
                        Term::contextual_closure(Term::ClosureContextValue(fields), signature)?
                    } else {
                        signature
                    })
                }
            }
        }
        if matches!(context, InputContext::Unknown) {
            return Ok(());
        }
        let mut trial = self.engine.trial();
        let mut components = Vec::new();
        let mut fuel = budget.steps;
        let settled = self.inspect_component(target.clone())?;
        if let Some(settled) = &settled {
            nir::type_computations::charge_term(settled, &mut fuel)
                .map_err(|_| Error::Match(MatchError::BudgetExhausted))?;
        }
        let template = build(
            context,
            settled.as_ref(),
            &mut trial,
            &mut components,
            0,
            0,
            &mut fuel,
        )?;
        let (mut engine, _) = trial
            .match_many(
                &[(
                    target,
                    I::Instantiate {
                        template,
                        arguments: components,
                    },
                )],
                Budget {
                    steps: fuel,
                    ..budget
                },
            )
            .map_err(Error::Match)?;
        engine.advance(
            fuel,
            Budget {
                steps: fuel,
                ..budget
            },
        );
        if engine.has_refuted_obligation() {
            return Err(Error::TypeView(
                "input context constraint was refuted".into(),
            ));
        }
        self.engine = engine;
        Ok(())
    }

    /// Refine settled components using definition equalities established by
    /// the caller (for example, evidence-backed associated Type selection).
    /// No inference variable or unsettled obligation is exported as a fact.
    pub fn refine_type_views(
        &mut self,
        mut resolve: impl FnMut(&Term, Budget) -> Result<Term, String>,
        budget: Budget,
    ) -> Result<(), Error> {
        let I::Instantiate { template, .. } = &self.signature else {
            return Err(Error::InvalidSignature);
        };
        let Term::Function { parameters, .. } = template else {
            return Err(Error::InvalidSignature);
        };
        let allowance = Budget {
            steps: budget.steps / 2 / (parameters.len() + 1),
            ..budget
        };
        if allowance.steps == 0 {
            return Err(Error::TypeView("call type-view budget exhausted".into()));
        }
        self.refine_type_views_metered(
            |ty, remaining| {
                remaining.steps = remaining
                    .steps
                    .checked_sub(allowance.steps)
                    .ok_or("call type-view budget exhausted")?;
                resolve(ty, allowance)
            },
            budget,
        )
    }

    /// Share unused query work between parameter views. All equalities remain
    /// transactional: a failed later query publishes none of the earlier views.
    pub fn refine_type_views_metered(
        &mut self,
        mut resolve: impl FnMut(&Term, &mut Budget) -> Result<Term, String>,
        budget: Budget,
    ) -> Result<(), Error> {
        let I::Instantiate { template, .. } = &self.signature else {
            return Err(Error::InvalidSignature);
        };
        let Term::Function { parameters, .. } = template else {
            return Err(Error::InvalidSignature);
        };
        let count = parameters.len();
        let mut queries = Budget {
            steps: budget.steps / 2,
            ..budget
        };
        if queries.steps == 0 {
            return Err(Error::TypeView("call type-view budget exhausted".into()));
        }
        let mut fuel = budget.steps / 2;
        let mut changes = Vec::new();
        for index in 0..=count {
            let term = if index == count {
                self.result.clone()
            } else {
                self.parameter_term(index)?
            };
            let Some(before) = self.inspect_component(term)? else {
                continue;
            };
            crate::associated::charge_projection_copy(&before, &mut fuel)
                .map_err(Error::TypeView)?;
            let available = queries.steps;
            let after = resolve(&before, &mut queries).map_err(Error::TypeView)?;
            if queries.steps > available {
                return Err(Error::TypeView(
                    "type-view resolver increased its budget".into(),
                ));
            }
            if before != after {
                crate::associated::charge_projection_copy(&after, &mut fuel)
                    .map_err(Error::TypeView)?;
                changes.push((before, after));
            }
        }
        // Publish only after every requested query succeeds.
        if !changes.is_empty() {
            for (before, after) in changes {
                self.engine
                    .assume_type_equality(before.clone(), after.clone())
                    .map_err(Error::Obligations)?;
                self.views.insert(before, after);
            }
            self.engine.advance(count + 2, budget);
        }
        Ok(())
    }
    pub fn type_argument(&self, index: usize) -> Result<Option<Term>, Error> {
        self.inspect_component(
            self.types
                .get(index)
                .ok_or(Error::TypeArgumentCount)?
                .clone(),
        )
    }
    /// The current candidate's input sort, after dependent-domain staging.
    pub fn type_argument_sort(&self, index: usize) -> Result<Sort, Error> {
        self.parameter_sorts
            .get(index)
            .copied()
            .ok_or(Error::TypeArgumentCount)
    }
    pub fn constrain_type_argument(
        &mut self,
        index: usize,
        value: &Term,
        budget: Budget,
    ) -> Result<(), Error> {
        let input = self
            .types
            .get(index)
            .ok_or(Error::TypeArgumentCount)?
            .clone();
        if self.parameter_sorts.get(index) == Some(&Sort::ComptimeValue) {
            let mut fuel = budget.steps;
            nir::type_computations::charge_term(value, &mut fuel)
                .map_err(|_| Error::Match(MatchError::BudgetExhausted))?;
            if value.sort() != Sort::ComptimeValue {
                return Err(Error::InvalidSignature);
            }
            let mut engine = self.engine.trial();
            engine
                .check_context_type(value)
                .map_err(Error::Obligations)?;
            match engine.inspect(&input).map_err(Error::Obligations)? {
                I::Meta(variable) => engine
                    .assign(variable, I::Stable(value.clone()))
                    .map_err(Error::Obligations)?,
                retained => {
                    let retained = engine.inspect_type(&retained).map_err(Error::Obligations)?;
                    nir::type_computations::charge_term(&retained, &mut fuel)
                        .map_err(|_| Error::Match(MatchError::BudgetExhausted))?;
                    if retained != *value {
                        let relations = retained
                            .index_type_relations(value)
                            .ok_or(Error::InvalidSignature)?;
                        let relations: Vec<_> = relations
                            .into_iter()
                            .map(|(a, b)| (I::Stable(a.clone()), I::Stable(b.clone())))
                            .collect();
                        engine = engine
                            .match_many(
                                &relations,
                                Budget {
                                    steps: fuel,
                                    ..budget
                                },
                            )
                            .map_err(Error::Match)?
                            .0;
                    }
                }
            }
            engine.advance(
                fuel,
                Budget {
                    steps: fuel,
                    ..budget
                },
            );
            self.engine = engine;
            return Ok(());
        }
        let (mut engine, _) = self
            .engine
            .match_many(&[(input, I::Stable(value.clone()))], budget)
            .map_err(Error::Match)?;
        // Matching can wake every pre-existing relation that used this slot.
        engine.advance(budget.steps, budget);
        self.engine = engine;
        Ok(())
    }
    /// Constrain this isolated candidate using equations under its generic
    /// Type-parameter frame. These are obligations, never caller assumptions.
    /// Failure leaves the candidate unchanged.
    pub fn constrain_type_equations(
        &mut self,
        equations: &[(Term, Term)],
        budget: Budget,
    ) -> Result<(), Error> {
        let mut fuel = budget.steps;
        let frames = [self.parameter_sorts.clone()];
        for (left, right) in equations {
            for term in [left, right] {
                nir::type_computations::charge_term(term, &mut fuel)
                    .map_err(|_| Error::Match(MatchError::BudgetExhausted))?;
                term.check_scope(&frames)?;
            }
        }
        let relations = equations
            .iter()
            .map(|(left, right)| {
                (
                    I::Instantiate {
                        template: left.clone(),
                        arguments: self.types.clone(),
                    },
                    I::Instantiate {
                        template: right.clone(),
                        arguments: self.types.clone(),
                    },
                )
            })
            .collect::<Vec<_>>();
        let budget = Budget {
            steps: fuel,
            ..budget
        };
        let (mut engine, _) = self
            .engine
            .match_many(&relations, budget)
            .map_err(Error::Match)?;
        engine.advance(budget.steps, budget);
        self.engine = engine;
        Ok(())
    }
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
    /// A settled parameter context may not depend on every inferred Type
    /// parameter. Unrelated unresolved slots must not prevent its use.
    fn parameter_term(&self, index: usize) -> Result<I, Error> {
        let I::Instantiate { template, .. } = &self.signature else {
            return Err(Error::InvalidSignature);
        };
        let Term::Function { parameters, .. } = template else {
            return Err(Error::InvalidSignature);
        };
        let parameter = parameters.get(index).ok_or(Error::ArgumentCount)?;
        Ok(I::Instantiate {
            template: parameter.ty.substitute(&[])?,
            arguments: self.types.clone(),
        })
    }
    fn inspect_component(&self, term: I) -> Result<Option<Term>, Error> {
        match self.engine.inspect_type(&term) {
            Ok(value) => Ok(Some(self.views.get(&value).cloned().unwrap_or(value))),
            Err(ObligationError::Inference(crate::InferenceError::Unresolved(_))) => Ok(None),
            Err(error) => Err(Error::Obligations(error)),
        }
    }
    /// A checking context can be available while unrelated proof obligations
    /// still wait. `finish` is the only instance export and checks all of them.
    pub fn parameter_type(&self, index: usize) -> Result<Option<Term>, Error> {
        self.inspect_component(self.parameter_term(index)?)
    }
    /// A description may inspect a settled result without exporting a call
    /// instance. Unresolved binders remain private to this candidate.
    pub fn result_type(&self) -> Result<Option<Term>, Error> {
        self.inspect_component(self.result.clone())
    }
    pub fn parameter_input_context(&self, index: usize) -> Result<InputContext, Error> {
        self.input_context(self.parameter_term(index)?, &mut 100_000, 0)
    }
    fn input_context(
        &self,
        term: I,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<InputContext, Error> {
        if depth >= 128 || *fuel == 0 {
            return Err(Error::TypeView(
                "input checking context budget exhausted".into(),
            ));
        }
        *fuel -= 1;
        if let Some(ty) = self.inspect_component(term.clone())? {
            return Ok(InputContext::Known(ty));
        }
        let shape = self.engine.inspect(&term).map_err(Error::Obligations)?;
        let (template, arguments) = match shape {
            I::Instantiate {
                template,
                arguments,
            } => (template, Some(arguments)),
            I::Stable(template) => (template, None),
            I::Meta(id) => {
                for &index in &self.dependent_value_parameters {
                    *fuel = fuel.checked_sub(1).ok_or_else(|| {
                        Error::TypeView("input checking context budget exhausted".into())
                    })?;
                    if matches!(self.engine.inspect(&self.types[index]).map_err(Error::Obligations)?, I::Meta(other) if other == id)
                    {
                        return Ok(InputContext::UnknownDependentValue);
                    }
                }
                return Ok(InputContext::Unknown);
            }
        };
        let mut field_context = |field: Term| -> Result<InputContext, Error> {
            let sort = field.sort();
            let field = match &arguments {
                Some(arguments) => I::Instantiate {
                    template: field,
                    arguments: arguments.clone(),
                },
                None => I::Stable(field),
            };
            let context = self.input_context(field, fuel, depth + 1)?;
            Ok(
                if sort == Sort::ComptimeValue && matches!(context, InputContext::Unknown) {
                    InputContext::UnknownValue
                } else {
                    context
                },
            )
        };
        match template {
            Term::Optional(payload) => {
                Ok(InputContext::Optional(Box::new(field_context(*payload)?)))
            }
            Term::ErrorResult { errors, result } => {
                let mut completed = Vec::new();
                for label in errors {
                    let InputContext::Known(label) = field_context(label)? else {
                        return Ok(InputContext::Unknown);
                    };
                    completed.push(label);
                }
                Ok(InputContext::ErrorResult {
                    errors: completed,
                    result: Box::new(field_context(*result)?),
                })
            }
            Term::Computation {
                mut effects,
                result,
            } => {
                for effect in &mut effects {
                    for argument in &mut effect.arguments {
                        let InputContext::Known(completed) = field_context(argument.clone())?
                        else {
                            return Ok(InputContext::Unknown);
                        };
                        *argument = completed;
                    }
                }
                Ok(InputContext::Computation {
                    effects,
                    result: Box::new(field_context(*result)?),
                })
            }
            Term::Nominal {
                declaration,
                arguments,
            } if self.engine.is_nominal_constructor(declaration) => Ok(InputContext::Nominal {
                declaration,
                arguments: arguments
                    .into_iter()
                    .map(&mut field_context)
                    .collect::<Result<_, _>>()?,
            }),
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments: inputs,
            } if self.engine.is_tuple_head(head) => {
                let [Term::TypeListValue(fields)] = inputs.as_slice() else {
                    return Ok(InputContext::Unknown);
                };
                Ok(InputContext::NormalTuple {
                    head,
                    fields: fields
                        .iter()
                        .cloned()
                        .map(&mut field_context)
                        .collect::<Result<_, _>>()?,
                })
            }
            Term::Tuple(fields) => Ok(InputContext::Tuple(
                fields
                    .into_iter()
                    .map(&mut field_context)
                    .collect::<Result<_, _>>()?,
            )),
            Term::Record(fields) => Ok(InputContext::Record(
                fields
                    .into_iter()
                    .map(|(name, ty)| field_context(ty).map(|context| (name, context)))
                    .collect::<Result<_, _>>()?,
            )),
            _ => Ok(self
                .callable_context_with_budget(&term, fuel, depth + 1)?
                .map(InputContext::Callable)
                .unwrap_or(InputContext::Unknown)),
        }
    }

    pub fn parameter_callable_context(
        &self,
        index: usize,
    ) -> Result<Option<CallableContext>, Error> {
        self.callable_context(&self.parameter_term(index)?)
    }
    fn callable_context(&self, term: &I) -> Result<Option<CallableContext>, Error> {
        self.callable_context_with_budget(term, &mut 100_000, 0)
    }
    fn callable_context_with_budget(
        &self,
        term: &I,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<CallableContext>, Error> {
        if depth >= 128 || *fuel == 0 {
            return Err(Error::TypeView(
                "input checking context budget exhausted".into(),
            ));
        }
        *fuel -= 1;
        let shape = self.engine.inspect(term).map_err(Error::Obligations)?;
        let (template, arguments) = match shape {
            I::Instantiate {
                template,
                arguments,
            } => (template, Some(arguments)),
            I::Stable(template) => (template, None),
            I::Meta(_) => return Ok(None),
        };
        let comptime = template.is_comptime_callable();
        let Term::Function { parameters, result } = template.callable_signature() else {
            return Ok(None);
        };
        // These callbacks must not depend on their own runtime inputs.
        let component = |ty: &Term| -> Result<Option<Term>, Error> {
            let template = ty.substitute(&[])?;
            self.inspect_component(match &arguments {
                Some(arguments) => I::Instantiate {
                    template,
                    arguments: arguments.clone(),
                },
                None => I::Stable(template),
            })
        };
        let parameters = parameters
            .iter()
            .map(|parameter| {
                if parameter.sort != Sort::Runtime {
                    return Err(Error::InvalidSignature);
                }
                component(&parameter.ty)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut open_environment = false;
        let environment = if let Term::Closure { environment, .. } = &template {
            Some(
                environment
                    .iter()
                    .map(|ty| {
                        self.inspect_component(match &arguments {
                            Some(arguments) => I::Instantiate {
                                template: ty.clone(),
                                arguments: arguments.clone(),
                            },
                            None => I::Stable(ty.clone()),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else if let Term::ContextualClosure { context, .. } = &template {
            let term = match &arguments {
                Some(arguments) => I::Instantiate {
                    template: context.as_ref().clone(),
                    arguments: arguments.clone(),
                },
                None => I::Stable(context.as_ref().clone()),
            };
            match self.engine.inspect(&term).map_err(Error::Obligations)? {
                I::Stable(Term::ClosureContextValue(fields)) => {
                    Some(fields.into_iter().map(Some).collect())
                }
                I::Instantiate {
                    template: Term::ClosureContextValue(fields),
                    arguments,
                } => Some(
                    fields
                        .into_iter()
                        .map(|template| {
                            self.inspect_component(I::Instantiate {
                                template,
                                arguments: arguments.clone(),
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                I::Meta(_)
                | I::Stable(
                    Term::Bound {
                        sort: Sort::ComptimeValue,
                        ..
                    }
                    | Term::Rigid {
                        sort: Sort::ComptimeValue,
                        ..
                    }
                    | Term::NormalForm {
                        sort: Sort::ComptimeValue,
                        ..
                    },
                )
                | I::Instantiate {
                    template:
                        Term::NormalForm {
                            sort: Sort::ComptimeValue,
                            ..
                        },
                    ..
                } => {
                    open_environment = true;
                    None
                }
                _ => return Err(Error::InvalidSignature),
            }
        } else {
            None
        };
        let result_type = component(&result)?;
        let result_context = if result_type.is_none() {
            let template = result.substitute(&[])?;
            let term = match &arguments {
                Some(arguments) => I::Instantiate {
                    template,
                    arguments: arguments.clone(),
                },
                None => I::Stable(template),
            };
            match self.input_context(term, fuel, depth + 1)? {
                InputContext::Unknown => None,
                context => Some(Box::new(context)),
            }
        } else {
            None
        };
        Ok(Some(CallableContext {
            comptime,
            environment,
            open_environment,
            parameters,
            result: result_type,
            result_context,
        }))
    }
    /// Obtain stable inferred binders without expanding the whole signature.
    /// Effect formation applies its own expansion budget before that copy.
    pub fn inferred_type_arguments(&self) -> Result<Vec<Term>, Error> {
        if self.engine.waits_only_for_inference() {
            return Err(Error::NeedsContext);
        }
        self.types
            .iter()
            .map(|ty| self.engine.freeze(ty).map_err(Error::Obligations))
            .collect()
    }

    pub fn finish(self) -> Result<Instance, Error> {
        if self.engine.waits_only_for_inference() {
            return Err(Error::NeedsContext);
        }
        let signature = self.engine.freeze(&self.signature).map_err(|cause| {
            if let ObligationError::Unsettled(id) = &cause {
                if let Some((requirement, status)) = self.engine.describe_requirement(*id) {
                    return Error::UnsettledRequirement {
                        cause,
                        requirement,
                        status,
                    };
                }
            }
            Error::Obligations(cause)
        })?;
        let result = self
            .engine
            .freeze(&self.result)
            .map_err(Error::Obligations)?;
        let type_arguments = self
            .types
            .iter()
            .map(|ty| self.engine.freeze(ty).map_err(Error::Obligations))
            .collect::<Result<_, _>>()?;
        Ok(Instance {
            type_arguments,
            signature,
            result,
        })
    }
}
impl std::fmt::Display for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(call-instance (type-arguments")?;
        for ty in &self.type_arguments {
            write!(f, " {ty}")?;
        }
        write!(
            f,
            ") (signature {}) (result {}))",
            self.signature, self.result
        )
    }
}
