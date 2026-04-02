use std::sync::Arc;

fn main() {
    fn demo() -> (Arc<i32>, impl Fn() -> Arc<i32>) {
        let a = Arc::new(5);

        let a_c = Arc::clone(&a);

        let f = move || (&a_c).clone();

        (a, f)
    }

    let (x, f) = demo();
}
