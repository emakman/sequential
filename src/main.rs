use sequential::actor;

actor! {
    #[derive(Clone, Copy)]
    struct Accountant<T: 'static + Send + Sync + Copy + Default + std::ops::AddAssign + std::ops::SubAssign> {
        x: T,
    }
    impl<T: Send + Sync + Copy + Default + std::ops::AddAssign + std::ops::SubAssign> Accountant<T> {
        fn new(x: T, y: usize) -> Self {
            let _ = y;
            Self {
                x,
            }
        }
        #[action]
        async fn increase(&mut self, n: T) -> T {
            self.x += n;
            self.x
        }
        #[action]
        async fn decrease(&mut self, n: T) -> T {
            self.x -= n;
            self.x
        }
        #[action(generator)]
        async fn count_to_ten_on_demand(&self) -> usize {
            for i in 1..=10 {
                println!("counting {i}...");
                yield i;
            }
        }
        #[action(multi)]
        async fn count_to_ten_immediately(&self) -> usize {
            for i in 1..=10 {
                println!("counting {i}...");
                yield i;
            }
        }
    }
}

#[tokio::main]
async fn main() {
    use tokio_stream::StreamExt;
    let acct = AccountantHandle::new(7, 8);
    let _seq1 = acct.count_to_ten_immediately().await;
    println!("{}", acct.increase(12).await);
    let mut seq = acct.count_to_ten_on_demand().await;
    println!("No work yet...");
    println!("{:?}", seq.next().await);
    println!("pause...");
    println!("{:?}", seq.next().await);
    println!("pause...");
    println!("{:?}", seq.next().await);
    let acct = seq.unwrap();
    println!("{}", acct.increase(3).await);
    println!("{}", acct.decrease(7).await);
    println!("Hello, world!");
}
