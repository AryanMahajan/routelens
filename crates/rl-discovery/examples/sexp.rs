use rl_discovery::{Language, SourceIndex};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    let lang = Language::of(std::path::Path::new(&path)).unwrap();
    let mut index = SourceIndex::new().unwrap();
    let file = index.parse(path.clone(), lang, source).unwrap();
    println!("{}", file.root().to_sexp());
}
