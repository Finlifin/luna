//! Bounded observation and freezing; no assignment or comptime invocation.
use super::*;
fn charge(fuel:&mut usize,amount:usize)->Result<(),InferenceError> {
    *fuel=fuel.checked_sub(amount).ok_or(InferenceError::Budget)?;
    Ok(())
}
fn term_work(term:&Term,fuel:&mut usize,max_depth:usize)->Result<usize,InferenceError> {
    let before=*fuel;
    nir::type_computations::charge_term_with_depth(term,fuel,max_depth).map_err(|_|InferenceError::Budget)?;
    Ok(before-*fuel)
}
impl InferenceContext {
    pub fn unresolved_metered(&self,term:&InferenceTerm,fuel:&mut usize)->Result<BTreeSet<MetaId>,InferenceError> {
        self.unresolved_with_limits(term,fuel,128)
    }
    pub fn unresolved_with_limits(&self,term:&InferenceTerm,fuel:&mut usize,max_depth:usize)->Result<BTreeSet<MetaId>,InferenceError> {
        let mut pending=vec![term];
        let mut seen=BTreeSet::new();
        let mut result=BTreeSet::new();
        while let Some(term)=pending.pop() {
            charge(fuel,1)?;
            match term {
                InferenceTerm::Stable(term)=>{term_work(term,fuel,max_depth)?;}
                InferenceTerm::Meta(id) if seen.insert(*id)=>{
                    let variable=self.variables.get(id).ok_or(InferenceError::ForeignVariable(*id))?;
                    if let Some(value)=&variable.value {pending.push(value);} else {result.insert(*id);}
                }
                InferenceTerm::Meta(_)=>{}
                InferenceTerm::Instantiate {template,arguments}=>{
                    term_work(template,fuel,max_depth)?;
                    for index in template.substitution_parameters().map_err(InferenceError::InvalidSubstitution)?.into_keys() {
                        charge(fuel,1)?;
                        pending.push(arguments.get(index).ok_or(InferenceError::InvalidSubstitution(TypeError::MissingArgument(index)))?);
                    }
                }
            }
        }
        Ok(result)
    }

    /// Charge alias traversal, copies and an upper bound on substitution work
    /// before allocating expanded terms. Unused inputs are never evaluated.
    pub fn freeze_metered(&self,term:&InferenceTerm,fuel:&mut usize)->Result<Term,InferenceError> {
        self.freeze_with_limits(term,fuel,128)
    }
    pub fn freeze_with_limits(&self,term:&InferenceTerm,fuel:&mut usize,max_depth:usize)->Result<Term,InferenceError> {
        enum Work<'a> { Visit(&'a InferenceTerm), Cache(MetaId), Substitute(&'a Term,usize,Vec<usize>,usize) }
        let mut work=vec![Work::Visit(term)];
        let mut values:Vec<(Term,usize)>=Vec::new();
        let mut cache:BTreeMap<MetaId,(Term,usize)>=BTreeMap::new();
        while let Some(action)=work.pop() {
            charge(fuel,1)?;
            match action {
                Work::Visit(InferenceTerm::Stable(term))=>{
                    let size=term_work(term,fuel,max_depth)?;
                    values.push((term.clone(),size));
                }
                Work::Visit(InferenceTerm::Meta(id))=>{
                    if let Some((value,size))=cache.get(id) {
                        charge(fuel,*size)?;
                        values.push((value.clone(),*size));
                    } else {
                        let variable=self.variables.get(id).ok_or(InferenceError::ForeignVariable(*id))?;
                        let value=variable.value.as_ref().ok_or(InferenceError::Unresolved(*id))?;
                        work.push(Work::Cache(*id));work.push(Work::Visit(value));
                    }
                }
                Work::Visit(InferenceTerm::Instantiate {template,arguments})=>{
                    let size=term_work(template,fuel,max_depth)?;
                    let indices=template.substitution_parameters().map_err(InferenceError::InvalidSubstitution)?.into_keys().collect::<Vec<_>>();
                    charge(fuel,indices.len())?;
                    work.push(Work::Substitute(template,arguments.len(),indices.clone(),size));
                    for index in indices.into_iter().rev() {
                        work.push(Work::Visit(arguments.get(index).ok_or(InferenceError::InvalidSubstitution(TypeError::MissingArgument(index)))?));
                    }
                }
                Work::Cache(id)=>{
                    let (value,size)=values.last().expect("visited value");
                    charge(fuel,*size)?;
                    cache.insert(id,(value.clone(),*size));
                }
                Work::Substitute(template,arity,indices,size)=>{
                    charge(fuel,arity)?;
                    let total=values[values.len()-indices.len()..].iter().try_fold(1usize,|n,(_,size)|n.checked_add(*size)).ok_or(InferenceError::Budget)?;
                    charge(fuel,size.checked_mul(total).ok_or(InferenceError::Budget)?)?;
                    let mut arguments=vec![None;arity];
                    for index in indices.into_iter().rev() {arguments[index]=Some(values.pop().expect("visited input").0);}
                    let result=template.substitute_partial(&arguments).map_err(InferenceError::InvalidSubstitution)?;
                    let size=term_work(&result,fuel,max_depth)?;
                    values.push((result,size));
                }
            }
        }
        Ok(values.pop().expect("visited root").0)
    }
}
