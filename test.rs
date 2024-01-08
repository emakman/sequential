struct AccountantHandle < T : Copy + Default + std :: ops :: AddAssign + std
:: ops :: SubAssign >
(:: tokio :: sync :: mpsc :: UnboundedSender < AccountantMessage < T > >) ;
const _ : () =
{
    struct Accountant < T : Copy + Default + std :: ops :: AddAssign + std ::
    ops :: SubAssign > { x : T, } impl < T : Copy + Default + std :: ops ::
    AddAssign + std :: ops :: SubAssign > Accountant < T >
    {
        fn init() -> Self { Self { x : Default :: default(), } } async fn
        increase(& mut self, n : T, output : :: tokio :: sync :: oneshot ::
        Sender < T >)
        { let _ = output.send((| | { { self.x += n ; self.x } }) ()) ; } async
        fn
        decrease(& mut self, n : T, output : :: tokio :: sync :: oneshot ::
        Sender < T >)
        { let _ = output.send((| | { { self.x -= n ; self.x } }) ()) ; } async
        fn
        count_to_ten(& self, output : :: tokio :: sync :: mpsc :: Sender <
        usize >)
        {
            if output.is_closed() { return } for i in 1 ..= 10
            {
                {
                    let _ = output.send(i).await ; if output.is_closed()
                    { return }
                } ;
            }
        }
    } impl < T : Copy + Default + std :: ops :: AddAssign + std :: ops ::
    SubAssign > AccountantHandle < T >
    {
        fn new() -> Self
        {
            let(tx, rx) = :: tokio :: sync :: mpsc :: unbounded_channel() ; ::
            tokio ::
            spawn(async move
            {
                let mut actor = struct Accountant < T : Copy + Default + std
                :: ops :: AddAssign + std :: ops :: SubAssign > { x : T, } ::
                init() ; let let Some(msg) = rx.recv().await { match msg {} }
            }) ; Self(tx)
        }
    }
} ; #[allow(non_camel_case_types)] enum AccountantMessage < T : Copy + Default
+ std :: ops :: AddAssign + std :: ops :: SubAssign >
{
    increase(T, :: tokio :: sync :: oneshot :: Sender < T >),
    decrease(T, :: tokio :: sync :: oneshot :: Sender < T >),
    count_to_ten(:: tokio :: sync :: mpsc :: Sender < usize >)
}
