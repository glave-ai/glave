/// Evaluate a reverse-Polish-notation expression over i64 and the operators
/// `+ - * /`.
pub fn eval_rpn(tokens: &[&str]) -> i64 {
    let mut stack: Vec<i64> = Vec::new();
    for &t in tokens {
        match t {
            "+" | "-" | "*" | "/" => {
                let b = stack.pop().unwrap_or(0);
                let a = stack.pop().unwrap_or(0);
                let r = match t {
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" => a / b,
                    _ => 0,
                };
                stack.push(r);
            }
            _ => stack.push(t.parse().unwrap_or(0)),
        }
    }
    stack.pop().unwrap_or(0)
}
