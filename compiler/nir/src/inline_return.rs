//! Lower a logical inlined-function return boundary into existing owned-value
//! and loop control nodes. The caller supplies fresh identities and registers
//! the returned private enum before running ordinary verification.
use crate::{DeclarationId, Primitive as P, Term};
use crate::program::{self as n, Expr, ExprKind as K, LocalId, SourceRange};

pub struct Boundary {
    pub packet: DeclarationId,
    pub slot: LocalId,
    pub exit: LocalId,
    pub result: LocalId,
    pub unreachable: LocalId,
}
impl std::fmt::Display for Boundary {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
        write!(f,"(inline-return-boundary (packet {}) (slot {}) (exit {}) (result {}) (unreachable {}))",
            self.packet,self.slot,self.exit,self.result,self.unreachable)
    }
}

struct Lower<'a> {
    ids: Boundary,
    packet: Term,
    fuel: &'a mut usize,
    sources: &'a mut Vec<n::ExpansionSource>,
    site: SourceRange,
    empty_replacements: Vec<SourceRange>,
}
impl Lower<'_> {
    fn node(&mut self,ty:Term,kind:K,original:SourceRange)->Result<Expr,String> {
        self.tick()?;
        let index=u32::try_from(self.sources.len()).map_err(|_|"inline return source space exhausted")?;
        self.sources.push(n::ExpansionSource {original,site:self.site});
        Ok(Expr {ty,kind,source:SourceRange::generated(index)})
    }
    fn tick(&mut self)->Result<(),String> {
        *self.fuel=self.fuel.checked_sub(1).ok_or("inline return budget exhausted")?;
        Ok(())
    }
    fn deliver(&mut self,value:Expr)->Result<Expr,String> {
        let origin=value.source;
        let ready=self.node(self.packet.clone(),K::Variant {declaration:self.ids.packet,index:1,fields:vec![value]},origin)?;
        let assign=self.node(Term::Primitive(P::Void),K::Assign {
            place:n::Place {root:self.ids.slot,fields:vec![]},value:Box::new(ready)},origin)?;
        self.empty_replacements.push(assign.source);
        let exit=self.node(Term::Primitive(P::Never),K::Break(self.ids.exit),origin)?;
        self.node(Term::Primitive(P::Never),K::Block {statements:vec![assign],result:Some(Box::new(exit))},origin)
    }
}
impl Lower<'_> {
    fn rewrite_returns(&mut self,body:&mut Expr,depth:usize)->Result<(),String> {
        if depth>=64 {return Err("inline return capture nesting budget exhausted".into());}
        let mut fuel=*self.fuel;
        let result=crate::visit::expression_mut_pruned(body,&mut fuel,|expr,remaining| {
            *self.fuel=*remaining;
            let result=(|| {
                match &mut expr.kind {
                    K::Closure {captures,..}=>{
                        // Captures execute in this boundary; the closure body
                        // owns a separate return target.
                        for capture in captures {
                            if let Some(value)=&mut capture.value {self.rewrite_returns(value,depth+1)?;}
                        }
                        Ok(false)
                    }
                    K::Return(value)=>{
                        let value=match value.take() {
                            Some(value)=>*value,
                            None=>self.node(Term::Primitive(P::Void),K::Unit,expr.source)?,
                        };
                        *expr=self.deliver(value)?;
                        // Visit the moved operand too: nested returns still
                        // target this boundary. No Return remains at this node.
                        Ok(true)
                    }
                    _=>Ok(true),
                }
            })();
            *remaining=*self.fuel;
            result
        });
        *self.fuel=fuel;
        result
    }
}

/// `body` is already type checked and hygienically renamed. No dummy value of
/// its result type is manufactured: the private slot starts in an empty case.
/// Failure discards the private input and rolls back added source provenance.
pub fn lower(body:Expr,result:Term,ids:Boundary,sources:&mut Vec<n::ExpansionSource>,
    site:SourceRange,fuel:&mut usize)->Result<(Expr,n::Enum,Vec<SourceRange>),String> {
    let checkpoint=sources.len();
    let lowered=lower_inner(body,result,ids,sources,site,fuel);
    if lowered.is_err() {sources.truncate(checkpoint);}
    lowered
}

fn lower_inner(mut body:Expr,result:Term,ids:Boundary,sources:&mut Vec<n::ExpansionSource>,
    site:SourceRange,fuel:&mut usize)->Result<(Expr,n::Enum,Vec<SourceRange>),String> {
    if std::collections::BTreeSet::from([ids.slot,ids.exit,ids.result,ids.unreachable]).len()!=4 {
        return Err("inline return boundary requires distinct local identities".into());
    }
    crate::type_computations::charge_term(&result,fuel).map_err(|e|e.to_string())?;
    if !result.runtime_dependencies().is_empty() || !result.substitution_parameters().map_err(|failure|failure.to_string())?.is_empty() {
        return Err("inline return result requires a closed declaration-scoped type".into());
    }
    let parameters=result.rigid_parameters().map_err(|failure|failure.to_string())?;
    if parameters.values().any(|sort|*sort!=crate::Sort::Type) {
        return Err("inline return runtime-dependent result storage is not yet implemented".into());
    }
    let type_parameters=parameters.keys().copied().collect::<Vec<_>>();
    let arguments=type_parameters.iter().map(|declaration|Term::Rigid {declaration:*declaration,sort:crate::Sort::Type}).collect();
    let packet=Term::Nominal {declaration:ids.packet,arguments};
    let descriptor=n::Enum {declaration:ids.packet,name:"<inline-return-packet>".into(),
        type_parameters,capabilities:vec![],requirements:vec![],
        variants:vec![n::Variant {name:"pending".into(),shape:n::VariantShape::Tuple,fields:vec![]},
            n::Variant {name:"ready".into(),shape:n::VariantShape::Tuple,fields:vec![result.clone()]}],
        groups:vec![],encoding:None,symbol_encoding:None};
    let mut lower=Lower {ids,packet,fuel,sources,site,empty_replacements:vec![]};
    lower.rewrite_returns(&mut body,0)?;
    let pending=lower.node(lower.packet.clone(),K::Variant {declaration:lower.ids.packet,index:0,fields:vec![]},site)?;
    let init=lower.node(Term::Primitive(P::Void),K::Let {binding:lower.ids.slot,value:Box::new(pending)},site)?;
    let delivered=lower.deliver(body)?;
    let region=lower.node(Term::Primitive(P::Void),K::Loop {identity:lower.ids.exit,body:Box::new(delivered)},site)?;
    let slot=lower.node(lower.packet.clone(),K::Local(lower.ids.slot),site)?;
    let value=lower.node(result.clone(),K::Local(lower.ids.result),site)?;
    // Every finite region exit delivers ready. Keep the impossible empty case
    // divergent without inventing a value or adding an unchecked extraction.
    let unit=lower.node(Term::Primitive(P::Void),K::Unit,site)?;
    let unreachable=lower.node(Term::Primitive(P::Never),K::Loop {identity:lower.ids.unreachable,body:Box::new(unit)},site)?;
    let selected=lower.node(result.clone(),K::Match {value:Box::new(slot),arms:vec![
        n::MatchArm {pattern:n::Pattern::Variant {declaration:lower.ids.packet,index:1,
            fields:vec![n::Pattern::Binding {binding:lower.ids.result,inner:Box::new(n::Pattern::Wild)}]},
            guard:None,body:value},
        n::MatchArm {pattern:n::Pattern::Wild,guard:None,body:unreachable},
    ]},site)?;
    let root=lower.node(result,K::Block {statements:vec![init,region],result:Some(Box::new(selected))},site)?;
    Ok((root,descriptor,lower.empty_replacements))
}
