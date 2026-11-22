// L'interface de la bibliothèque (Slint) est compilée dans le binaire.
fn main() {
    slint_build::compile("ui/library.slint").expect("ui/library.slint");
}
