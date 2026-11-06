//! Owned CLI/MCP HTTP runtime and a process-wide native DNS admission bound.
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::{io, net::{SocketAddr, ToSocketAddrs}, sync::{Arc, OnceLock, atomic::{AtomicUsize, Ordering}}};

pub(crate) struct Runtime(Option<tokio::runtime::Runtime>);
impl Runtime {
    pub(crate) fn new() -> Result<Self, String> {
        tokio::runtime::Builder::new_current_thread().enable_all().build().map(|runtime|Self(Some(runtime))).map_err(|_|"cannot start bounded HTTP observer".into())
    }
}
impl std::ops::Deref for Runtime {
    type Target = tokio::runtime::Runtime;
    fn deref(&self) -> &Self::Target {self.0.as_ref().unwrap()}
}
impl Drop for Runtime {
    fn drop(&mut self) {if let Some(runtime)=self.0.take(){runtime.shutdown_background();}}
}
struct Lease(Arc<AtomicUsize>);
impl Lease {
    fn take(counter:Arc<AtomicUsize>)->io::Result<Self>{
        counter.fetch_update(Ordering::AcqRel,Ordering::Acquire,|current|(current<4).then_some(current+1)).map_err(|_|io::Error::new(io::ErrorKind::WouldBlock,"HTTP observer DNS capacity reached"))?;
        Ok(Self(counter))
    }
}
impl Drop for Lease {fn drop(&mut self){self.0.fetch_sub(1,Ordering::AcqRel);}}
struct Resolver {leases:Arc<AtomicUsize>}
impl Resolver {
    fn lookup(&self, work:impl FnOnce()->io::Result<Vec<SocketAddr>>+Send+'static)->Resolving {
        let counter=self.leases.clone();
        Box::pin(async move {
            let lease=Lease::take(counter)?;
            // The blocking closure owns the lease, not the canceled HTTP future or its dropped runtime.
            // At most four system resolver calls can remain alive across repeated MCP invocations.
            let addresses=tokio::task::spawn_blocking(move ||{let _lease=lease;work()}).await.map_err(|_|io::Error::other("HTTP observer DNS worker failed"))??;
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}
impl Resolve for Resolver {
    fn resolve(&self,name:Name)->Resolving {
        let host=name.as_str().to_owned();
        self.lookup(move ||(host.as_str(),0).to_socket_addrs().map(|addresses|addresses.take(64).collect()))
    }
}
pub(crate) fn client(builder:reqwest::ClientBuilder)->reqwest::ClientBuilder {
    static LEASES:OnceLock<Arc<AtomicUsize>>=OnceLock::new();
    builder.dns_resolver(Arc::new(Resolver{leases:LEASES.get_or_init(||Arc::new(AtomicUsize::new(0))).clone()}))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration,Instant};
    #[test]
    fn canceled_lookups_keep_native_leases_across_runtime_teardown_without_a_queue(){
        let count=Arc::new(AtomicUsize::new(0));let resolver=Resolver{leases:count.clone()};let mut releases=Vec::new();
        let runtime=Runtime::new().unwrap();
        for _ in 0..4 {
            let (release,wait)=std::sync::mpsc::channel();let(started,ready)=std::sync::mpsc::channel();releases.push(release);
            let future=resolver.lookup(move ||{started.send(()).unwrap();let _=wait.recv_timeout(Duration::from_secs(3));Ok(vec![])});
            runtime.block_on(async{assert!(tokio::time::timeout(Duration::from_millis(20),future).await.is_err());});
            ready.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        assert_eq!(count.load(Ordering::Acquire),4);
        let refused=runtime.block_on(resolver.lookup(||panic!("fifth lookup must never start")));
        assert!(refused.is_err());
        let before=Instant::now();drop(runtime);assert!(before.elapsed()<Duration::from_millis(200));
        assert_eq!(count.load(Ordering::Acquire),4,"runtime drop cannot release native work admission");
        for release in releases {release.send(()).unwrap();}
        let deadline=Instant::now()+Duration::from_secs(1);while count.load(Ordering::Acquire)!=0 && Instant::now()<deadline {std::thread::sleep(Duration::from_millis(1));}
        assert_eq!(count.load(Ordering::Acquire),0);
    }
}
