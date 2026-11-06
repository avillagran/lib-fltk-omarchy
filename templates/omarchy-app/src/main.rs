// {{name}}: count things from the keyboard.
//
// The binary is a thin shell: `{{name}}` (or `{{name}} open`) opens the
// window, every other word is a command that needs no display
// (`{{name}} help`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit({{crate}}::cli::run(&args));
}
