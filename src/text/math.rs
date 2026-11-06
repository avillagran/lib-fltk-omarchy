//! TeX math as readable text: the subset agents write in chat answers
//! (Greek letters, operators, relations, arrows, `^`/`_` scripts, `\frac`,
//! `\sqrt`, `\mathbb`, accents, `\text`), turned into Unicode — the same
//! approach as o3-code's `latex` module, without a math layout engine.
//!
//! Scripts use Unicode superscript and subscript characters when every
//! character has one (`x^2` → `x²`, `a_i` → `aᵢ`), else `^(…)` / `_(…)`.
//! Fractions become `a⁄b` (parenthesized when not simple), roots `√(x)`.
//! Unknown commands stay as written.

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α", "beta" => "β", "gamma" => "γ", "delta" => "δ", "epsilon" => "ϵ", "varepsilon" => "ε",
        "zeta" => "ζ", "eta" => "η", "theta" => "θ", "vartheta" => "ϑ", "iota" => "ι", "kappa" => "κ",
        "lambda" => "λ", "mu" => "μ", "nu" => "ν", "xi" => "ξ", "pi" => "π", "varpi" => "ϖ", "rho" => "ρ",
        "varrho" => "ϱ", "sigma" => "σ", "varsigma" => "ς", "tau" => "τ", "upsilon" => "υ", "phi" => "ϕ",
        "varphi" => "φ", "chi" => "χ", "psi" => "ψ", "omega" => "ω",
        "Gamma" => "Γ", "Delta" => "Δ", "Theta" => "Θ", "Lambda" => "Λ", "Xi" => "Ξ", "Pi" => "Π",
        "Sigma" => "Σ", "Upsilon" => "Υ", "Phi" => "Φ", "Psi" => "Ψ", "Omega" => "Ω",
        "sum" => "∑", "prod" => "∏", "coprod" => "∐", "int" => "∫", "iint" => "∬", "iiint" => "∭", "oint" => "∮",
        "infty" => "∞", "partial" => "∂", "nabla" => "∇", "forall" => "∀", "exists" => "∃", "nexists" => "∄",
        "emptyset" => "∅", "varnothing" => "∅", "in" => "∈", "notin" => "∉", "ni" => "∋", "subset" => "⊂",
        "supset" => "⊃", "subseteq" => "⊆", "supseteq" => "⊇", "cup" => "∪", "cap" => "∩", "bigcup" => "⋃",
        "bigcap" => "⋂", "setminus" => "∖", "land" => "∧", "wedge" => "∧", "lor" => "∨", "vee" => "∨",
        "neg" => "¬", "lnot" => "¬", "top" => "⊤", "bot" => "⊥",
        "le" => "≤", "leq" => "≤", "ge" => "≥", "geq" => "≥", "ne" => "≠", "neq" => "≠", "approx" => "≈",
        "equiv" => "≡", "sim" => "∼", "simeq" => "≃", "cong" => "≅", "propto" => "∝", "ll" => "≪", "gg" => "≫",
        "pm" => "±", "mp" => "∓", "times" => "×", "div" => "÷", "cdot" => "·", "ast" => "∗", "star" => "⋆",
        "circ" => "∘", "bullet" => "•", "oplus" => "⊕", "otimes" => "⊗", "odot" => "⊙",
        "to" => "→", "rightarrow" => "→", "leftarrow" => "←", "gets" => "←", "leftrightarrow" => "↔",
        "Rightarrow" => "⇒", "Leftarrow" => "⇐", "Leftrightarrow" => "⇔", "implies" => "⟹", "iff" => "⟺",
        "mapsto" => "↦", "uparrow" => "↑", "downarrow" => "↓", "longrightarrow" => "⟶", "longleftarrow" => "⟵",
        "ldots" => "…", "dots" => "…", "cdots" => "⋯", "vdots" => "⋮", "ddots" => "⋱",
        "langle" => "⟨", "rangle" => "⟩", "lceil" => "⌈", "rceil" => "⌉", "lfloor" => "⌊", "rfloor" => "⌋",
        "lbrace" => "{", "rbrace" => "}", "{" => "{", "}" => "}", "|" => "‖", "Vert" => "‖", "vert" => "|", "mid" => "∣",
        "angle" => "∠", "perp" => "⊥", "parallel" => "∥", "degree" => "°", "prime" => "′", "hbar" => "ℏ",
        "ell" => "ℓ", "Re" => "ℜ", "Im" => "ℑ", "aleph" => "ℵ", "therefore" => "∴", "because" => "∵",
        "%" => "%", "$" => "$", "&" => "&", "#" => "#", "_" => "_",
        "," | ":" | ";" | "quad" | "qquad" | " " | "!" => " ",
        _ => return None,
    })
}

fn operator(name: &str) -> bool {
    matches!(
        name,
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "log" | "ln" | "exp" | "lim" | "max" | "min" | "sup" | "inf" | "det"
            | "dim" | "ker" | "gcd" | "arg" | "deg" | "sinh" | "cosh" | "tanh" | "arcsin" | "arccos" | "arctan" | "Pr"
            | "mod" | "bmod"
    )
}

fn blackboard(c: char) -> char {
    match c {
        'R' => 'ℝ',
        'N' => 'ℕ',
        'Z' => 'ℤ',
        'Q' => 'ℚ',
        'C' => 'ℂ',
        'P' => 'ℙ',
        'H' => 'ℍ',
        other => other,
    }
}

fn superscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴', '5' => '⁵', '6' => '⁶', '7' => '⁷', '8' => '⁸', '9' => '⁹',
        '+' => '⁺', '-' | '−' => '⁻', '=' => '⁼', '(' => '⁽', ')' => '⁾',
        'a' => 'ᵃ', 'b' => 'ᵇ', 'c' => 'ᶜ', 'd' => 'ᵈ', 'e' => 'ᵉ', 'f' => 'ᶠ', 'g' => 'ᵍ', 'h' => 'ʰ', 'i' => 'ⁱ',
        'j' => 'ʲ', 'k' => 'ᵏ', 'l' => 'ˡ', 'm' => 'ᵐ', 'n' => 'ⁿ', 'o' => 'ᵒ', 'p' => 'ᵖ', 'r' => 'ʳ', 's' => 'ˢ',
        't' => 'ᵗ', 'u' => 'ᵘ', 'v' => 'ᵛ', 'w' => 'ʷ', 'x' => 'ˣ', 'y' => 'ʸ', 'z' => 'ᶻ',
        'T' => 'ᵀ', '′' => '′', '*' | '∗' => '*',
        _ => return None,
    })
}

fn subscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀', '1' => '₁', '2' => '₂', '3' => '₃', '4' => '₄', '5' => '₅', '6' => '₆', '7' => '₇', '8' => '₈', '9' => '₉',
        '+' => '₊', '-' | '−' => '₋', '=' => '₌', '(' => '₍', ')' => '₎',
        'a' => 'ₐ', 'e' => 'ₑ', 'h' => 'ₕ', 'i' => 'ᵢ', 'j' => 'ⱼ', 'k' => 'ₖ', 'l' => 'ₗ', 'm' => 'ₘ', 'n' => 'ₙ',
        'o' => 'ₒ', 'p' => 'ₚ', 'r' => 'ᵣ', 's' => 'ₛ', 't' => 'ₜ', 'u' => 'ᵤ', 'v' => 'ᵥ', 'x' => 'ₓ',
        _ => return None,
    })
}

/// `body` as a script: Unicode script characters when all exist, else
/// `^(body)` (or `^x` for one character).
fn script(body: &str, sup: bool) -> String {
    let map = if sup { superscript } else { subscript };
    let mapped: Option<String> = body.chars().map(map).collect();
    match mapped {
        Some(s) if !s.is_empty() => s,
        _ => {
            let mark = if sup { '^' } else { '_' };
            if body.chars().count() == 1 { format!("{mark}{body}") } else { format!("{mark}({body})") }
        }
    }
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn command(&mut self) -> String {
        let start = self.at;
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                    self.at += 1;
                }
            }
            Some(_) => self.at += 1,
            None => {}
        }
        self.chars[start..self.at].iter().collect()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    /// One argument: a `{group}`, a `\command` or one character.
    fn argument(&mut self) -> String {
        self.skip_space();
        match self.peek() {
            Some('{') => {
                self.at += 1;
                self.group(Some('}'))
            }
            Some('\\') => {
                self.at += 1;
                let name = self.command();
                self.expand(&name)
            }
            Some(c) => {
                self.at += 1;
                self.one(c)
            }
            None => String::new(),
        }
    }

    /// An argument taken literally (`\text{…}`).
    fn raw_argument(&mut self) -> String {
        self.skip_space();
        if self.peek() != Some('{') {
            return self.peek().map(|c| {
                self.at += 1;
                c.to_string()
            }).unwrap_or_default();
        }
        self.at += 1;
        let (start, mut level) = (self.at, 1);
        while let Some(c) = self.peek() {
            match c {
                '{' => level += 1,
                '}' => {
                    level -= 1;
                    if level == 0 {
                        break;
                    }
                }
                _ => {}
            }
            self.at += 1;
        }
        let s: String = self.chars[start..self.at.min(self.chars.len())].iter().collect();
        self.at += 1;
        s
    }

    fn one(&self, c: char) -> String {
        match c {
            '-' => "−".into(),
            '*' => "∗".into(),
            '\'' => "′".into(),
            '~' | '&' => " ".into(),
            c => c.to_string(),
        }
    }

    fn expand(&mut self, name: &str) -> String {
        if self.depth > 64 {
            return String::new();
        }
        self.depth += 1;
        let out = match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let (a, b) = (self.argument(), self.argument());
                let wrap = |s: &str| {
                    if s.chars().count() <= 2 || !s.contains([' ', '+', '−', '-']) { s.to_string() } else { format!("({s})") }
                };
                format!("{}⁄{}", wrap(&a), wrap(&b))
            }
            "sqrt" => {
                self.skip_space();
                let root = if self.peek() == Some('[') {
                    self.at += 1;
                    let start = self.at;
                    while self.peek().is_some_and(|c| c != ']') {
                        self.at += 1;
                    }
                    let r: String = self.chars[start..self.at].iter().collect();
                    self.at += 1;
                    script(&r, true)
                } else {
                    String::new()
                };
                let body = self.argument();
                if body.chars().count() <= 1 { format!("{root}√{body}") } else { format!("{root}√({body})") }
            }
            "text" | "textrm" | "mathrm" | "operatorname" | "textnormal" | "mbox" | "mathtt" | "texttt" => self.raw_argument(),
            "mathbf" | "textbf" | "boldsymbol" | "bm" | "mathit" | "textit" | "mathcal" | "mathscr" | "mathfrak" | "mathsf" => self.argument(),
            "mathbb" => self.raw_argument().chars().map(blackboard).collect(),
            "overline" | "bar" => format!("{}\u{0305}", self.argument()),
            "underline" => format!("{}\u{0332}", self.argument()),
            "hat" | "widehat" => format!("{}\u{0302}", self.argument()),
            "tilde" | "widetilde" => format!("{}\u{0303}", self.argument()),
            "vec" => format!("{}\u{20D7}", self.argument()),
            "dot" => format!("{}\u{0307}", self.argument()),
            "ddot" => format!("{}\u{0308}", self.argument()),
            "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "displaystyle" | "textstyle"
            | "limits" | "nolimits" => {
                self.skip_space();
                if matches!(name, "left" | "right") && self.peek() == Some('.') {
                    self.at += 1;
                }
                String::new()
            }
            "begin" | "end" => {
                self.raw_argument();
                String::new()
            }
            "\\" | "cr" | "newline" => "\n".into(),
            _ if operator(name) => name.to_string(),
            _ => symbol(name).map(str::to_string).unwrap_or_else(|| format!("\\{name}")),
        };
        self.depth -= 1;
        out
    }

    fn group(&mut self, close: Option<char>) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                c if Some(c) == close => return out,
                '{' => {
                    let g = self.group(Some('}'));
                    out.push_str(&g);
                }
                '}' => {}
                '\\' => {
                    let name = self.command();
                    let e = self.expand(&name);
                    out.push_str(&e);
                }
                '^' => {
                    let a = self.argument();
                    out.push_str(&script(&a, true));
                }
                '_' => {
                    let a = self.argument();
                    out.push_str(&script(&a, false));
                }
                c if c.is_whitespace() => {
                    if !out.ends_with(' ') && !out.is_empty() {
                        out.push(' ');
                    }
                }
                c => out.push_str(&self.one(c)),
            }
        }
        out
    }
}

/// One formula as text (`display`: its own block; the text is the same).
pub fn to_text(tex: &str) -> String {
    let mut p = Parser { chars: tex.trim().chars().collect(), at: 0, depth: 0 };
    let body = p.group(None);
    let mut out = String::with_capacity(body.len());
    for c in body.trim().chars() {
        if !(c == ' ' && out.ends_with(' ')) {
            out.push(c);
        }
    }
    out
}

/// Rewrites `\[..\]` and `\(..\)` outside code to `$$..$$` and `$..$`, so
/// the Markdown math extension sees them (as T3's remark-math setup does).
pub fn normalize_delimiters(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut fence: Option<String> = None;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(open) = &fence {
            if trimmed.starts_with(open.as_str()) {
                fence = None;
            }
            out.push_str(line);
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(trimmed[..3].to_string());
            out.push_str(line);
            continue;
        }
        let mut in_code = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '`' {
                in_code = !in_code;
                out.push(c);
                continue;
            }
            if c == '\\' && !in_code {
                match chars.peek() {
                    Some('[') | Some(']') => {
                        chars.next();
                        out.push_str("$$");
                        continue;
                    }
                    Some('(') | Some(')') => {
                        chars.next();
                        out.push('$');
                        continue;
                    }
                    _ => {}
                }
            }
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_constructs() {
        assert_eq!(to_text("x^2 + y^2 = z^2"), "x² + y² = z²");
        assert_eq!(to_text("a_i + a_{n+1}"), "aᵢ + aₙ₊₁");
        assert_eq!(to_text("e^{i\\pi} + 1 = 0"), "e^(iπ) + 1 = 0");
        assert_eq!(to_text("\\frac{a}{b}"), "a⁄b");
        assert_eq!(to_text("\\frac{x+1}{2}"), "(x+1)⁄2");
        assert_eq!(to_text("\\sqrt{2}"), "√2");
        assert_eq!(to_text("\\sqrt[3]{x+y}"), "³√(x+y)");
        assert_eq!(to_text("\\alpha \\leq \\beta \\to \\infty"), "α ≤ β → ∞");
        assert_eq!(to_text("\\mathbb{R}^n"), "ℝⁿ");
        assert_eq!(to_text("\\sum_{i=1}^{n} i"), "∑ᵢ₌₁ⁿ i");
        assert_eq!(to_text("\\sin x + \\cos y"), "sin x + cos y");
        assert_eq!(to_text("\\text{if } x > 0"), "if x > 0");
        assert_eq!(to_text("\\left( a \\right)"), "( a )");
        assert_eq!(to_text("\\unknown"), "\\unknown");
        assert_eq!(to_text("x^{\\gamma}"), "x^γ");
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let deep = "\\frac{".repeat(200) + &"}".repeat(200);
        let _ = to_text(&deep);
    }

    #[test]
    fn delimiters_outside_code() {
        assert_eq!(normalize_delimiters("a \\(x\\) b \\[y\\]"), "a $x$ b $$y$$");
        assert_eq!(normalize_delimiters("`\\(x\\)`"), "`\\(x\\)`");
        assert_eq!(normalize_delimiters("```\n\\[x\\]\n```\n"), "```\n\\[x\\]\n```\n");
    }
}
