mod basic;
mod comptime;
mod diagnostic;
mod lex;
mod parse;
mod query;
mod typing;
mod vfs;

use std::path::Path;

use diagnostic::{DiagnosticContext, FlurryError};

use crate::{basic::create_source_map, lex::lex, parse::parser::Parser};

// fn main() {
// // 创建源映射
// let source_map = crate::basic::create_source_map();

// // 从命令行参数获取文件名，如果没有则使用默认文件
// let args: Vec<String> = std::env::args().collect();
// let filename = if args.len() > 1 {
//     &args[1]
// } else {
//     "test_errors.tao"
// };
// // 加载测试文件
// let source_file = match source_map.load_file(&Path::new(filename)) {
//     Ok(file) => file,
//     Err(_) => panic!("MAN!"),
// };

// // 创建诊断上下文
// let mut diag_ctx = DiagnosticContext::new(&source_map);

// // 演示lexer错误处理
// demonstrate_lexer_errors(&mut diag_ctx, &source_file);
// }

// fn demonstrate_lexer_errors(
//     diag_ctx: &mut DiagnosticContext,
//     source_file: &std::sync::Arc<rustc_span::SourceFile>,
// ) {
//     // 创建lexer并处理token
//     let (_, errors) = lex(&source_file.src.as_ref().unwrap(), source_file.start_pos);
//     for err in errors {
//         err.emit(diag_ctx, source_file.start_pos);
//     }
// }

// fn main() {
//     let mut ast = parse::ast::Ast::new();
//     let lhs = NodeBuilder::new(NodeKind::Int, DUMMY_SP).build(&mut ast);
//     let rhs = NodeBuilder::new(NodeKind::Int, DUMMY_SP).build(&mut ast);
//     let result = NodeBuilder::new(NodeKind::Add, DUMMY_SP)
//         .add_single_child(lhs)
//         .add_single_child(rhs)
//         .build(&mut ast);

//     let elem1 = NodeBuilder::new(NodeKind::Int, DUMMY_SP).build(&mut ast);
//     let elem2 = NodeBuilder::new(NodeKind::Int, DUMMY_SP).build(&mut ast);
//     let elem3 = NodeBuilder::new(NodeKind::Int, DUMMY_SP).build(&mut ast);
//     let elem4 = result;
//     let list = NodeBuilder::new(NodeKind::ListOf, DUMMY_SP)
//         .add_multiple_children(vec![elem1, elem2, elem3, elem4])
//         .build(&mut ast);

//     std::fs::write("ast.lisp", ast.dump_to_s_expression(list, ())).unwrap();
//     // dbg!(result, ast);
// }

fn main() {
    let source_map = create_source_map();
    // 从命令行参数获取文件名，如果没有则使用默认文件
    let args: Vec<String> = std::env::args().collect();
    let filename = if args.len() > 1 { &args[1] } else { "test.fl" };
    // 加载测试文件
    let source_file = match source_map.load_file(&Path::new(filename)) {
        Ok(file) => file,
        Err(_) => panic!("MAN!"),
    };

    // 创建诊断上下文
    let mut diag_ctx = DiagnosticContext::new(&source_map);

    let (tokens, errors) = lex(&source_file.src.as_ref().unwrap(), source_file.start_pos);
    for err in errors {
        err.emit(&mut diag_ctx, source_file.start_pos);
    }

    let mut parser = Parser::new(&source_map, tokens);
    parser.parse(&mut diag_ctx);
    let ast = parser.ast;

    std::fs::write("ast.lisp", ast.dump_to_s_expression(ast.root, ())).unwrap();
}
