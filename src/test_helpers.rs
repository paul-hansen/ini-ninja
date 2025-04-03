#[macro_export]
macro_rules! assert_eq_preserve_new_lines {
    ($left:expr, $right:expr $(,)?) => ({
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    panic!(
                        indoc::indoc!("\
                            Assertion failed
                            Expected:
                            ```
                            {}
                            ```
                            Got:
                            ```
                            {}
                            ```"
                        ),
                        *right_val,
                        *left_val,
                    )
                }
            }
        }
    });
    ($left:expr, $right:expr, $($arg:tt)+) => ({
        match (&$left, &$right) {
            (left_val, right_val) => {
                if !(*left_val == *right_val) {
                    panic!(
                        indoc::indoc!("\
                            Assertion failed: {}
                            Expected:
                            ```
                            {}
                            ```
                            Got:
                            ```
                            {}
                            ```"
                        ),
                        format_args!($($arg)+),
                        *right_val,
                        *left_val,
                    )
                }
            }
        }
    });
}
