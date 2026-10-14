pub fn wrap(package:&mut nir::program::Package,name:&str) {
    use nir::{DeclarationId,program::{Expr,ExprKind as K,LocalId},inline_return::{self,Boundary}};
    let function=package.functions.iter_mut().find(|f|f.name==name).unwrap();
    let origin=function.body.source;
    let body=std::mem::replace(&mut function.body,Expr {ty:nir::Term::Primitive(nir::Primitive::Void),
        source:origin,kind:K::Unit});
    let nir::Term::Function {result,..}=function.signature.callable_signature() else {panic!("signature")};
    let result=(**result).clone();
    let fresh=|index|LocalId {owner:u32::MAX,index};
    let ids=Boundary {packet:DeclarationId {package:u64::MAX-3,local:0},slot:fresh(10000),exit:fresh(10001),result:fresh(10002),unreachable:fresh(10003)};
    let site=body.source;
    let (body,descriptor,empty_replacements)=inline_return::lower(body,result,ids,&mut package.expansion_sources,site,&mut 100_000).unwrap();
    function.body=body;
    for source in empty_replacements {package.internal_return_empty_replacements.insert((source.start,source.end),descriptor.declaration);}
    package.internal_return_packets.insert(descriptor.declaration);
    package.enums.push(descriptor);
}
