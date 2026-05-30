/// Evaluate a reverse-Polish-notation expression over i64 and the operators
/// `+ - * /`.
pub fn eval_rpn(tokens: &[&str]) -> i64 {
    let mut stack: Vec<i64> = Vec::new();
    for &t in tokens {
        match t {
            "+" | "-" | "*" | "/" => {
                let b = stack.pop().unwrap_or(0);
                let a = stack.pop().unwrap_or(0);
                // BUG: non-commutative operators apply their operands in the
                // wrong order (b op a instead of a op b).
                let r = match t {
                    "+" => a + b,
                    "-" => b - a,
                    "*" => a * b,
                    "/" => b / a,
                    _ => 0,
                };
                stack.push(r);
            }
            _ => stack.push(t.parse().unwrap_or(0)),
        }
    }
    stack.pop().unwrap_or(0)
}
