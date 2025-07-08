// Re-export types from rustc_span
pub use rustc_span::{
    BytePos, Span,
    source_map::SourceMap,
    FileName, DUMMY_SP,
};

// rustc_span 需要全局设置，我们需要初始化 session globals
pub fn create_source_map() -> SourceMap {
    use rustc_span::source_map::FilePathMapping;
    SourceMap::new(FilePathMapping::empty())
}

