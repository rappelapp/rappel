//! The 15 test formulas from the spike request, verbatim.

/// Inline sentences, verbatim; `$…$` delimits math.
pub const INLINE: [&str; 4] = [
    r"1. Soit $f \in \mathcal{C}^1([a,b],\mathbb{R})$ et $\lVert f \rVert_\infty = \sup_{x \in [a,b]} |f(x)|$.",
    r"2. Le rayon de convergence vérifie $\frac{1}{R} = \limsup_{n \to +\infty} |a_n|^{1/n}$.",
    r"3. Le vecteur de Poynting $\overrightarrow{\Pi} = \frac{\vec{E} \wedge \vec{B}}{\mu_0}$ a pour flux la puissance rayonnée.",
    r"4. On a $n! \underset{n \to +\infty}{\sim} \sqrt{2\pi n}\left(\frac{n}{e}\right)^n$ (Stirling).",
];

/// Splits a sentence into (is_math, piece).
pub fn split(sentence: &str) -> Vec<(bool, &str)> {
    sentence
        .split('$')
        .enumerate()
        .filter(|(_, p)| !p.is_empty())
        .map(|(i, p)| (i % 2 == 1, p))
        .collect()
}

pub const DISPLAY: [&str; 11] = [
    r"\hat{f}(\xi) = \int_{-\infty}^{+\infty} f(x)\, e^{-2i\pi x\xi} \,\mathrm{d}x",
    r"\frac{1}{2\pi}\int_{0}^{2\pi} |f(t)|^2 \,\mathrm{d}t = \sum_{n=-\infty}^{+\infty} |c_n(f)|^2",
    r"f(x) = \sum_{k=0}^{n} \frac{f^{(k)}(a)}{k!}(x-a)^k + \int_a^x \frac{(x-t)^n}{n!} f^{(n+1)}(t)\,\mathrm{d}t",
    r"\begin{aligned} \operatorname{div} \vec{E} &= \frac{\rho}{\varepsilon_0} & \overrightarrow{\operatorname{rot}}\, \vec{E} &= -\frac{\partial \vec{B}}{\partial t} \\ \operatorname{div} \vec{B} &= 0 & \overrightarrow{\operatorname{rot}}\, \vec{B} &= \mu_0 \vec{j} + \mu_0\varepsilon_0 \frac{\partial \vec{E}}{\partial t} \end{aligned}",
    r"\frac{\partial u_{\text{em}}}{\partial t} + \operatorname{div} \overrightarrow{\Pi} = -\vec{j} \cdot \vec{E}",
    r"\begin{vmatrix} 1 & x_1 & \cdots & x_1^{n-1} \\ 1 & x_2 & \cdots & x_2^{n-1} \\ \vdots & \vdots & \ddots & \vdots \\ 1 & x_n & \cdots & x_n^{n-1} \end{vmatrix} = \prod_{1 \le i < j \le n} (x_j - x_i)",
    r"\mathbb{1}_{[0,1]}(x) = \begin{cases} 1 & \text{si } x \in [0,1], \\ 0 & \text{sinon.} \end{cases}",
    r"\left| \int_a^b f g \right| \le \left( \int_a^b f^2 \right)^{\frac{1}{2}} \left( \int_a^b g^2 \right)^{\frac{1}{2}}",
    r"\oiint_{\Sigma} \vec{E} \cdot \mathrm{d}\vec{S} = \frac{Q_{\text{int}}}{\varepsilon_0} \qquad \oint_{\mathcal{C}} \vec{B} \cdot \mathrm{d}\vec{\ell} = \mu_0 I_{\text{enlacé}}",
    r"i\hbar \frac{\partial \psi}{\partial t} = -\frac{\hbar^2}{2m} \Delta\psi + V\psi \qquad \frac{1}{1 + \cfrac{1}{1 + \cfrac{1}{x}}}",
    r"\exp(A) = \sum_{k=0}^{+\infty} \frac{A^k}{k!}, \quad P = \begin{pmatrix} \cos\theta & -\sin\theta \\ \sin\theta & \cos\theta \end{pmatrix}, \quad \binom{n}{k} = \frac{n!}{k!\,(n-k)!}",
];

/// Every formula: (label, source, display).
pub fn all() -> Vec<(String, &'static str, bool)> {
    let mut v = Vec::new();
    for (i, sentence) in INLINE.iter().enumerate() {
        let maths: Vec<_> = split(sentence).into_iter().filter(|p| p.0).collect();
        for (j, (_, m)) in maths.iter().enumerate() {
            let label = if maths.len() > 1 {
                format!("{}{}", i + 1, (b'a' + j as u8) as char)
            } else {
                (i + 1).to_string()
            };
            v.push((label, *m, false));
        }
    }
    for (i, f) in DISPLAY.iter().enumerate() {
        v.push(((i + 5).to_string(), *f, true));
    }
    v
}

pub const INVALID: [&str; 6] = [
    r"\frac{",
    r"\notacommand{x}",
    r"x^",
    r"\begin{aligned} a & b",
    r"}}}",
    r"\left( x",
];
