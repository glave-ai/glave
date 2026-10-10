use numkit::{dedupe, digits, fizzbuzz, gcd, title};

#[test]
fn fizzbuzz_names_multiples() {
    assert_eq!(fizzbuzz::fizzbuzz(0), "FizzBuzz");
    assert_eq!(fizzbuzz::fizzbuzz(7), "7");
    assert_eq!(fizzbuzz::fizzbuzz(9), "Fizz");
    assert_eq!(fizzbuzz::fizzbuzz(25), "Buzz");
    assert_eq!(fizzbuzz::fizzbuzz(45), "FizzBuzz");
}

#[test]
fn digits_add_up() {
    assert_eq!(digits::digit_sum(0), 0);
    assert_eq!(digits::digit_sum(907), 16);
    assert_eq!(digits::digit_sum(u64::MAX), 87);
}

#[test]
fn gcd_of_anything() {
    assert_eq!(gcd::gcd(0, 0), 0);
    assert_eq!(gcd::gcd(0, 9), 9);
    assert_eq!(gcd::gcd(12, 18), 6);
    assert_eq!(gcd::gcd(17, 5), 1);
}

#[test]
fn title_case() {
    assert_eq!(title::title("hELLO wORLD"), "Hello World");
    assert_eq!(title::title(""), "");
    assert_eq!(title::title("a  b"), "A  B");
}

#[test]
fn dedupe_keeps_first() {
    assert_eq!(dedupe::dedupe(&[3, 1, 3, 2, 1]), vec![3, 1, 2]);
    assert_eq!(dedupe::dedupe(&[]), Vec::<i32>::new());
}
