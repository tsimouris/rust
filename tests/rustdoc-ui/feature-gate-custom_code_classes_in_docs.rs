// check-pass

/// ```{class=language-c}
/// int main(void) { return 0; }
/// ```
//~^^^ WARNING custom classes in code blocks will change behaviour
//~| NOTE found these custom classes: class=language-c
//~| NOTE see issue #79483 <https://github.com/rust-lang/rust/issues/79483>
//~| HELP add `#![feature(custom_code_classes_in_docs)]` to the crate attributes to enable
pub struct Bar;
