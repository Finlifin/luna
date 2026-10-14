//! Select the once-only preparation producers needed by staged call inputs.
use super::*;
use std::collections::BTreeMap;

pub struct Preparation {
    pub indices: BTreeSet<usize>,
    pub expression: Expr,
    bindings: Vec<(usize, LocalId, Term)>,
}

pub fn preparation(expr: &Expr, fuel: &mut usize) -> Result<Option<Preparation>, String> {
    let Some((indices,bindings))=nir::preparation::inputs(expr,fuel)? else {return Ok(None)};
    let K::CallPreparation {statements,..}=&expr.kind else {unreachable!()};
    let values=bindings.iter().map(|(_,binding,ty)|Expr {source:expr.source,ty:ty.clone(),kind:K::Local(*binding)}).collect::<Vec<_>>();
    let ty=Term::Tuple(bindings.iter().map(|(_,_,ty)|ty.clone()).collect());
    let mut selected=Vec::new();
    for &index in &indices {
        crate::visit::expression(&statements[index],fuel,|_,_|Ok(()))?;
        selected.push(statements[index].clone());
    }
    let expression=Expr {source:expr.source,ty:ty.clone(),kind:K::Block {
        statements:selected,result:Some(Box::new(Expr {source:expr.source,ty,kind:K::Tuple(values)})),
    }};
    Ok(Some(Preparation {indices,expression,bindings}))
}

impl Preparation {
    pub fn apply(self, expr: &mut Expr, completed: Expr, fuel: &mut usize) -> Result<(),String> {
        self.apply_with_layouts(&[], expr, completed, fuel)
    }

    pub fn apply_with_layouts(self, heads: &[nir::normal_forms::Head], expr: &mut Expr, completed: Expr, fuel: &mut usize) -> Result<(),String> {
        if completed.ty!=self.expression.ty {return Err("completed preparation type mismatch".into())}
        let K::Tuple(values)=completed.kind else {return Err("completed preparation must be a tuple".into())};
        if values.len()!=self.bindings.len() {return Err("completed preparation arity mismatch".into())}
        let mut replacements=BTreeMap::new();
        let mut groups:BTreeMap<usize,Vec<Expr>>=BTreeMap::new();
        for ((index,binding,ty),value) in self.bindings.into_iter().zip(values) {
            if value.ty!=ty {return Err("completed preparation binding type mismatch".into())}
            constant(heads, &value,fuel,0)?;
            replacements.insert(binding,value.clone());
            groups.entry(index).or_default().push(Expr {source:value.source,ty:Term::Primitive(Primitive::Void),kind:K::Let {binding,value:Box::new(value)}});
        }
        let K::CallPreparation {statements,result:Some(call)}=&mut expr.kind else {return Err("preparation changed before application".into())};
        let K::Call {callee,arguments,..}=&mut call.kind else {return Err("preparation call changed before application".into())};
        let Term::Function {parameters,..}=callee.ty.callable_signature() else {return Err("preparation callee is not callable".into())};
        for argument in arguments {
            if parameters[argument.parameter].sort!=Sort::Runtime && !argument.is_default {
                argument.value=prepare_input_with_layouts(heads, &argument.value,&replacements,fuel)?;
            }
        }
        // Completed inputs needed only for instance selection must not leave
        // Type/Integer-only bindings in runtime storage. Reconstruct only data
        // still referenced by the runtime preparation and result call.
        let mut live=nir::preparation::references(call,fuel)?;
        for (index,statement) in statements.iter().enumerate() {
            if !self.indices.contains(&index) {live.extend(nir::preparation::references(statement,fuel)?);}
        }
        for group in groups.values_mut() {
            group.retain(|expr|matches!(&expr.kind,K::Let {binding,..} if live.contains(binding)));
        }
        let mut result=Vec::new();
        for (index,statement) in std::mem::take(statements).into_iter().enumerate() {
            if self.indices.contains(&index) {result.extend(groups.remove(&index).unwrap_or_default());}
            else {result.push(statement);}
        }
        *statements=result;
        Ok(())
    }
}
