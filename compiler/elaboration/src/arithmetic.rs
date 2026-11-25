//! Explicit source identities and contracts for arithmetic and unary operator protocols.
use nir::program::{MethodContract, Trait};
use nir::{AssociatedTypeOrigin, DeclarationId, Parameter, Primitive, Sort, Term};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Negate,
    Invert,
}

impl Operator {
    pub const ALL: [Self; 7] = [
        Self::Add,
        Self::Subtract,
        Self::Multiply,
        Self::Divide,
        Self::Remainder,
        Self::Negate,
        Self::Invert,
    ];

    pub fn is_unary(self) -> bool {
        matches!(self, Self::Negate | Self::Invert)
    }

    pub fn method(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Subtract => "sub",
            Self::Multiply => "mul",
            Self::Divide => "div",
            Self::Remainder => "rem",
            Self::Negate => "neg",
            Self::Invert => "invert",
        }
    }
}

impl std::fmt::Display for Operator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.method())
    }
}

/// Validate the bound declaration, never a same-spelled trait found by lookup.
pub fn protocol(
    traits: &[Trait],
    id: DeclarationId,
    operator: Operator,
) -> Result<&MethodContract, String> {
    let mut candidates = traits
        .iter()
        .filter(|interface| interface.declaration == id);
    let interface = candidates
        .next()
        .ok_or("arithmetic protocol is not a declared trait")?;
    if candidates.next().is_some() {
        return Err("duplicate arithmetic protocol identity".into());
    }
    let rhs = if operator.is_unary() {
        if !interface.type_parameters.is_empty() {
            return Err("unary operator protocol cannot have type parameters".into());
        }
        None
    } else {
        let [rhs] = interface.type_parameters.as_slice() else {
            return Err("arithmetic protocol requires one RHS type parameter".into());
        };
        Some(Term::Rigid {
            declaration: *rhs,
            sort: Sort::Type,
        })
    };
    let outputs = interface
        .associated
        .iter()
        .filter(|member| member.name == "Output")
        .collect::<Vec<_>>();
    let [output] = outputs.as_slice() else {
        return Err("arithmetic protocol requires one Output associated type".into());
    };
    if output.ty != Term::Primitive(Primitive::Type) {
        return Err("arithmetic Output must have type Type".into());
    }
    let methods = interface
        .methods
        .iter()
        .filter(|method| method.name == operator.method())
        .collect::<Vec<_>>();
    let [method] = methods.as_slice() else {
        return Err(format!(
            "arithmetic protocol requires one {} method",
            operator.method()
        ));
    };
    let subject = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let mut parameters = vec![Parameter {
        sort: Sort::Runtime,
        ty: subject.clone(),
    }];
    if let Some(rhs) = &rhs {
        parameters.push(Parameter {
            sort: Sort::Runtime,
            ty: rhs.clone(),
        });
    }
    let expected = Term::Function {
        parameters,
        result: Box::new(Term::AssociatedType {
            origin: Box::new(AssociatedTypeOrigin {
                interface: id,
                member: output.declaration,
            }),
            arguments: rhs.into_iter().collect(),
            subject: Box::new(subject),
        }),
    };
    if method.signature.callable_signature() != Ok(&expected)
        || !method.by_value_receiver
        || method.by_reference_receiver
        || method.parameter_bindings.len() != if operator.is_unary() { 1 } else { 2 }
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
    {
        return Err(if operator.is_unary() {
            format!(
                "unary operator protocol requires fn {}(self) -> Output",
                operator.method()
            )
        } else {
            format!(
                "arithmetic protocol requires fn {}(self, rhs: Rhs) -> Output",
                operator.method()
            )
        });
    }
    Ok(method)
}

/// Validate a binary operator protocol (e.g. `Implies` or `ComptimeImplies`)
/// that has one `.Rhs` type parameter, `assoc Output: Type`, and method
/// `fn <method_name>(self, rhs: Rhs) -> Output`.
pub fn binary_operator_protocol<'a>(
    traits: &'a [Trait],
    id: DeclarationId,
    method_name: &str,
    require_comptime_only: bool,
) -> Result<&'a MethodContract, String> {
    let mut candidates = traits
        .iter()
        .filter(|interface| interface.declaration == id);
    let interface = candidates
        .next()
        .ok_or_else(|| format!("{method_name} protocol is not a declared trait"))?;
    if candidates.next().is_some() {
        return Err(format!("duplicate {method_name} protocol identity"));
    }
    if require_comptime_only && !interface.comptime_only {
        return Err(format!(
            "{method_name} comptime protocol must be declared as a comptime trait"
        ));
    }
    let [rhs] = interface.type_parameters.as_slice() else {
        return Err(format!(
            "{method_name} protocol requires one RHS type parameter"
        ));
    };
    let rhs = Term::Rigid {
        declaration: *rhs,
        sort: Sort::Type,
    };
    let outputs = interface
        .associated
        .iter()
        .filter(|member| member.name == "Output")
        .collect::<Vec<_>>();
    let [output] = outputs.as_slice() else {
        return Err(format!(
            "{method_name} protocol requires one Output associated type"
        ));
    };
    if output.ty != Term::Primitive(Primitive::Type) {
        return Err(format!("{method_name} Output must have type Type"));
    }
    let methods = interface
        .methods
        .iter()
        .filter(|method| method.name == method_name)
        .collect::<Vec<_>>();
    let [method] = methods.as_slice() else {
        return Err(format!(
            "{method_name} protocol requires one {method_name} method"
        ));
    };
    let subject = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let expected = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Runtime,
                ty: subject.clone(),
            },
            Parameter {
                sort: Sort::Runtime,
                ty: rhs.clone(),
            },
        ],
        result: Box::new(Term::AssociatedType {
            origin: Box::new(AssociatedTypeOrigin {
                interface: id,
                member: output.declaration,
            }),
            arguments: vec![rhs],
            subject: Box::new(subject),
        }),
    };
    if method.signature.callable_signature() != Ok(&expected)
        || !method.by_value_receiver
        || method.by_reference_receiver
        || method.parameter_bindings.len() != 2
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
    {
        return Err(format!(
            "{method_name} protocol requires fn {method_name}(self, rhs: Rhs) -> Output"
        ));
    }
    Ok(method)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompoundOperator {
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    RemAssign,
}

impl CompoundOperator {
    pub const ALL: [Self; 5] = [
        Self::AddAssign,
        Self::SubAssign,
        Self::MulAssign,
        Self::DivAssign,
        Self::RemAssign,
    ];

    pub fn method(self) -> &'static str {
        match self {
            Self::AddAssign => "add_assign",
            Self::SubAssign => "sub_assign",
            Self::MulAssign => "mul_assign",
            Self::DivAssign => "div_assign",
            Self::RemAssign => "rem_assign",
        }
    }

    pub fn binary_operator(self) -> Operator {
        match self {
            Self::AddAssign => Operator::Add,
            Self::SubAssign => Operator::Subtract,
            Self::MulAssign => Operator::Multiply,
            Self::DivAssign => Operator::Divide,
            Self::RemAssign => Operator::Remainder,
        }
    }

    pub fn from_bin_op(op: hir::BinOp) -> Option<Self> {
        match op {
            hir::BinOp::Add => Some(Self::AddAssign),
            hir::BinOp::Sub => Some(Self::SubAssign),
            hir::BinOp::Mul => Some(Self::MulAssign),
            hir::BinOp::Div => Some(Self::DivAssign),
            hir::BinOp::Rem => Some(Self::RemAssign),
            _ => None,
        }
    }
}

pub fn compound_protocol<'a>(
    traits: &'a [Trait],
    id: DeclarationId,
    operator: CompoundOperator,
    comptime: bool,
) -> Result<&'a MethodContract, String> {
    let mut candidates = traits.iter().filter(|item| item.declaration == id);
    let interface = candidates
        .next()
        .ok_or_else(|| format!("{operator:?} binding is not a trait"))?;
    if candidates.next().is_some() {
        return Err(format!(
            "{operator:?} binding matches multiple trait declarations"
        ));
    }
    if interface.comptime_only != comptime {
        return Err(if comptime {
            format!("Comptime{operator:?} protocol must be declared as a pure comptime trait")
        } else {
            format!("{operator:?} protocol cannot be declared as a pure comptime trait")
        });
    }
    let [rhs] = interface.type_parameters.as_slice() else {
        return Err(format!(
            "{operator:?} protocol requires one Rhs trait parameter"
        ));
    };
    let name = operator.method();
    let mut methods = interface.methods.iter().filter(|m| m.name == name);
    let method = methods
        .next()
        .ok_or_else(|| format!("{operator:?} protocol requires a {name} method"))?;
    if methods.next().is_some() {
        return Err(format!(
            "{operator:?} protocol allows only one {name} method"
        ));
    }
    if method.is_quote {
        return Err(format!("{operator:?} {name} cannot be a quote function"));
    }
    if method.signature.is_comptime_callable() != comptime {
        return Err(if comptime {
            format!("Comptime{operator:?} {name} must be a comptime function")
        } else {
            format!("{operator:?} {name} must be a runtime function")
        });
    }
    let self_ty = Term::Rigid {
        declaration: interface.self_parameter,
        sort: Sort::Type,
    };
    let rhs_ty = Term::Rigid {
        declaration: *rhs,
        sort: Sort::Type,
    };
    let receiver_ty = if comptime {
        self_ty
    } else {
        Term::Pointer(Box::new(self_ty))
    };
    let expected = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Runtime,
                ty: receiver_ty,
            },
            Parameter {
                sort: Sort::Runtime,
                ty: rhs_ty,
            },
        ],
        result: Box::new(Term::Primitive(Primitive::Void)),
    };
    let valid_receiver = if comptime {
        method.by_value_receiver && !method.by_reference_receiver
    } else {
        !method.by_value_receiver && method.by_reference_receiver
    };
    if method.signature.callable_signature() != Ok(&expected)
        || !valid_receiver
        || method.parameter_bindings.len() != 2
        || !method.type_parameters.is_empty()
        || !method.type_arguments.is_empty()
    {
        return Err(if comptime {
            format!(
                "Comptime{operator:?} protocol requires comptime fn {name}(self, rhs: Rhs) -> void"
            )
        } else {
            format!("{operator:?} protocol requires fn {name}(*self, rhs: Rhs) -> void")
        });
    }
    Ok(method)
}
