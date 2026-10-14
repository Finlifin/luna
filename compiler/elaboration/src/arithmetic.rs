//! Explicit source identities and contracts for arithmetic and unary operator protocols.
use nir::{AssociatedTypeOrigin,DeclarationId,Parameter,Primitive,Sort,Term};
use nir::program::{Trait,MethodContract};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
#[repr(usize)]
pub enum Operator {Add,Subtract,Multiply,Divide,Remainder,Negate,Invert}
impl Operator {
    pub const ALL:[Self;7]=[Self::Add,Self::Subtract,Self::Multiply,Self::Divide,Self::Remainder,Self::Negate,Self::Invert];
    pub fn is_unary(self)->bool {matches!(self,Self::Negate|Self::Invert)}
    pub fn method(self)->&'static str {
        match self {Self::Add=>"add",Self::Subtract=>"sub",Self::Multiply=>"mul",Self::Divide=>"div",Self::Remainder=>"rem",Self::Negate=>"neg",Self::Invert=>"invert"}
    }
}
impl std::fmt::Display for Operator {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(self.method())}
}
/// Validate the bound declaration, never a same-spelled trait found by lookup.
pub fn protocol(traits:&[Trait],id:DeclarationId,operator:Operator)->Result<&MethodContract,String> {
    let mut candidates=traits.iter().filter(|interface|interface.declaration==id);
    let interface=candidates.next().ok_or("arithmetic protocol is not a declared trait")?;
    if candidates.next().is_some() {return Err("duplicate arithmetic protocol identity".into());}
    let rhs=if operator.is_unary() {
        if !interface.type_parameters.is_empty() {return Err("unary operator protocol cannot have type parameters".into());}
        None
    } else {
        let [rhs]=interface.type_parameters.as_slice() else {return Err("arithmetic protocol requires one RHS type parameter".into());};
        Some(Term::Rigid {declaration:*rhs,sort:Sort::Type})
    };
    let outputs=interface.associated.iter().filter(|member|member.name=="Output").collect::<Vec<_>>();
    let [output]=outputs.as_slice() else {return Err("arithmetic protocol requires one Output associated type".into());};
    if output.ty!=Term::Primitive(Primitive::Type) {return Err("arithmetic Output must have type Type".into());}
    let methods=interface.methods.iter().filter(|method|method.name==operator.method()).collect::<Vec<_>>();
    let [method]=methods.as_slice() else {return Err(format!("arithmetic protocol requires one {} method",operator.method()));};
    let subject=Term::Rigid {declaration:interface.self_parameter,sort:Sort::Type};
    let mut parameters=vec![Parameter {sort:Sort::Runtime,ty:subject.clone()}];
    if let Some(rhs)=&rhs {parameters.push(Parameter {sort:Sort::Runtime,ty:rhs.clone()});}
    let expected=Term::Function {
        parameters,
        result:Box::new(Term::AssociatedType {
            origin:Box::new(AssociatedTypeOrigin {interface:id,member:output.declaration}),
            arguments:rhs.into_iter().collect(),subject:Box::new(subject),
        }),
    };
    if method.signature.callable_signature()!=&expected || !method.by_value_receiver || method.by_reference_receiver
        || method.parameter_bindings.len()!=if operator.is_unary() {1} else {2} || !method.type_parameters.is_empty() || !method.type_arguments.is_empty() {
        return Err(if operator.is_unary() {format!("unary operator protocol requires fn {}(self) -> Output",operator.method())}
            else {format!("arithmetic protocol requires fn {}(self, rhs: Rhs) -> Output",operator.method())});
    }
    Ok(method)
}
