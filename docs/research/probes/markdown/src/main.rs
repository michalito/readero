use pulldown_cmark::{Parser,Options,html};
fn main(){
 let sample="# Example\n\n| Input | Result |\n| --- | --- |\n| a | b |\n\n- [x] Read\n\n```rust\nfn main() {}\n```\n\nFootnote[^1].\n\n[^1]: A note.\n\n[Related](./chapter.md#example)\n";
 let opts=Options::ENABLE_TABLES|Options::ENABLE_TASKLISTS|Options::ENABLE_FOOTNOTES|Options::ENABLE_STRIKETHROUGH;
 let spans:Vec<_>=Parser::new_ext(sample,opts).into_offset_iter().collect();
 let mut output=String::new();html::push_html(&mut output,Parser::new_ext(sample,opts));
 assert!(output.contains("<table>"));assert!(output.contains("type=\"checkbox\""));assert!(output.contains("language-rust"));assert!(output.contains("footnote"));assert!(output.contains("./chapter.md#example"));assert!(spans.iter().all(|(_,r)|sample.is_char_boundary(r.start)&&sample.is_char_boundary(r.end)));
 println!("GFM-style tables, task lists, fenced code, footnotes, relative links, UTF-8 source offsets: passed");
}
