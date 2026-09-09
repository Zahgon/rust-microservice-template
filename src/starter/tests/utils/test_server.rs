use ctor::dtor;
use std::thread::JoinHandle;
use std::time::Duration;
use testcontainers::core::IntoContainerPort;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tokio::runtime::Builder;
use tokio::sync::OnceCell;

const CONFIG_FILE_PATH: &str = "./../../";

#[macro_export]
macro_rules! prepare_test_environment {
    () => {{
        test_server::init().await;
        reqwest::Client::new()
    }};
}

pub struct Server {
    #[allow(dead_code)]
    server_handle: JoinHandle<()>,
    container: ContainerAsync<Postgres>,
}

impl Server {
    pub async fn start() -> Self {
        let container = Postgres::default()
            .with_db_name("rust_template_db")
            .with_mapped_port(5432, 5432.tcp())
            .with_tag("16-alpine")
            .start()
            .await
            .unwrap();

        // The server is driven by a runtime of its own so that it outlives the
        // per-test runtimes created by `#[tokio::test]`.
        let server_handle = std::thread::spawn(move || {
            let runtime = Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed to build server runtime");

            runtime.block_on(async {
                let server = starter::run_with_config(CONFIG_FILE_PATH)
                    .await
                    .expect("Failed to bind address");
                server.await.expect("Failed to run server");
            });
        });
        tokio::time::sleep(Duration::from_secs(1)).await;
        Server {
            server_handle,
            container,
        }
    }

    pub fn container(&self) -> &ContainerAsync<Postgres> {
        &self.container
    }
}

pub(crate) static TEST_SERVER_ONCE: OnceCell<Server> = OnceCell::const_new();

pub(crate) async fn init() {
    TEST_SERVER_ONCE.get_or_init(Server::start).await;
}

//see https://stackoverflow.com/questions/78969766/how-can-i-call-drop-in-a-tokio-static-oncelock-in-rust
#[dtor]
fn cleanup() {
    if let Some(server) = TEST_SERVER_ONCE.get() {
        let id = server.container().id();
        let _ = std::process::Command::new("docker")
            .arg("kill")
            .arg(id)
            .output();
    }
}
