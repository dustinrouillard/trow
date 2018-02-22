use std;
use std::path::Path;
use std::sync::{Arc, Mutex};

use grpcio;
use grpc;

use failure::Error;
use futures::Future;
use uuid::Uuid;

/// Struct implementing callbacks for the Frontend
///
/// _uploads_: a HashSet of all uuids that are currently being tracked
#[derive(Clone)]
pub struct BackendService {
    uploads: Arc<Mutex<std::collections::HashSet<Layer>>>,
}

impl BackendService {
    pub fn new() -> Self {
        BackendService { uploads: Arc::new(Mutex::new(std::collections::HashSet::new())) }
    }
}

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
pub struct Layer {
    pub name: String,
    pub repo: String,
    pub digest: String,
}

/// given a layer, it returns it's size on disk
fn get_size(layer: Layer) -> Result<u64, Error> {
    let path = construct_absolute_path(layer)?;
    std::fs::metadata(path.as_os_str())?;
    debug!("Getting length of file {:?}", path.as_os_str());
    let file = std::fs::File::open(path)?;
    file.metadata()
        .and_then(|metadata| Ok(metadata.len()))
        .map_err(|e| e.into())
}

/// Delete a file by uuid.
pub fn delete_blob_by_uuid(uuid: &str) -> bool {
    use std::fs;
    let path = format!(
        "data/scratch/{}",
        uuid
    );

    fs::remove_file(path).map(|_| true).unwrap_or(false)
}

/// Takes the digest, and constructs an absolute pathstring to the digest.
fn construct_absolute_path(layer: Layer) -> Result<Box<Path>, Error> {
    std::env::current_dir()
        .map(|cwd| {
            let absolute_dir = cwd.join(format!("data/layers/{}/{}/{}", layer.name, layer.repo, layer.digest));
            debug!("Absolute Path: {:?}", absolute_dir);
            absolute_dir.into_boxed_path()
        })
        .map_err(|e| e.into())
}

impl grpc::backend_grpc::Backend for BackendService {
    fn layer_exists(
        &self,
        ctx: grpcio::RpcContext,
        req: grpc::backend::Layer,
        sink: grpcio::UnarySink<grpc::backend::LayerExistsResult>,
    ) {
        let layer = Layer {
            name: req.get_name().to_owned(),
            repo: req.get_repo().to_owned(),
            digest: req.get_digest().to_owned(),
        };

        let mut resp = grpc::backend::LayerExistsResult::new();
        let _ = get_size(layer)
            .map(|length| {
                debug!("Success, building return object");
                resp.set_success(true);
                resp.set_length(length);
            })
            .map_err(|_| {
                debug!("Failure, building return object");
                resp.set_success(false);
            });

        let req = req.clone();
        let f = sink.success(resp).map_err(move |e| {
            warn!("failed to reply! {:?}, {:?}", req, e)
        });
        ctx.spawn(f);
    }

    fn gen_uuid(
        &self,
        ctx: grpcio::RpcContext,
        req: grpc::backend::Layer,
        sink: grpcio::UnarySink<grpc::backend::GenUuidResult>,
    ) {
        let mut resp = grpc::backend::GenUuidResult::new();
        let layer = Layer {
            name: req.get_name().to_owned(),
            repo: req.get_repo().to_owned(),
            digest: Uuid::new_v4().to_string(),
        };
        {
            self.uploads.lock().unwrap().insert(layer.clone());
            debug!("Hash Table: {:?}", self.uploads);
        }
        resp.set_uuid(layer.digest.to_owned());
        let f = sink.success(resp).map_err(
            move |e| warn!("failed to reply! {:?}", e),
        );
        ctx.spawn(f);
    }

    fn uuid_exists(
        &self,
        ctx: grpcio::RpcContext,
        req: grpc::backend::Layer,
        sink: grpcio::UnarySink<grpc::backend::Result>,
    ) {
        let mut resp = grpc::backend::Result::new();
        let set = self.uploads.lock().unwrap();
        let layer = Layer {
            name: req.get_name().to_owned(),
            repo: req.get_repo().to_owned(),
            digest: req.get_digest().to_owned(),
        };
        resp.set_success(set.contains(&layer));

        let f = sink.success(resp).map_err(
            move |e| warn!("failed to reply! {:?}", e),
        );
        ctx.spawn(f);
    }

    fn cancel_upload(
        &self,
        ctx: grpcio::RpcContext,
        req: grpc::backend::Layer,
        sink: grpcio::UnarySink<grpc::backend::Result>,
    ) {
        let mut resp = grpc::backend::Result::new();
        let mut set = self.uploads.lock().unwrap();
        let layer = Layer {
            name: req.get_name().to_owned(),
            repo: req.get_repo().to_owned(),
            digest: req.get_digest().to_owned(),
        };
        let _ = delete_blob_by_uuid(&layer.digest);
        resp.set_success(set.remove(&layer));

        let f = sink.success(resp).map_err(
            move |e| warn!("failed to reply! {:?}", e),
        );
        ctx.spawn(f);
    }

    fn delete_uuid(
        &self,
        ctx: grpcio::RpcContext,
        req: grpc::backend::Layer,
        sink: grpcio::UnarySink<grpc::backend::Result>) {
        let layer = Layer {
            name: req.get_name().to_owned(),
            repo: req.get_repo().to_owned(),
            digest: req.get_digest().to_owned(),
        };
        let mut set = self.uploads.lock().unwrap();

        let mut resp = grpc::backend::Result::new();
        debug!("Before Delete: {:?}", self.uploads);
        resp.set_success(set.remove(&layer));
        debug!("After Delete: {:?}", self.uploads);

        let f = sink.success(resp).map_err(
            move |e| warn!("failed to reply! {:?}", e),
        );
        ctx.spawn(f);
    }

    fn upload_manifest(
        &self,
        ctx: grpcio::RpcContext,
        _req: grpc::backend::Manifest,
        sink: grpcio::UnarySink<grpc::backend::Result>) {
        warn!("upload manifest not implemented");
        let mut resp = grpc::backend::Result::new();
        resp.set_success(false);

        let f = sink.success(resp).map_err(
            move |e| warn!("failed to reply! {:?}", e),
        );
        ctx.spawn(f);
    }

    // ---------- Admin calls ----------------

    fn get_uuids(
        &self,
        ctx: grpcio::RpcContext,
        _req: grpc::backend::Empty,
        sink: grpcio::UnarySink<grpc::backend::UuidList>,
    ) {
        let mut resp = grpc::backend::UuidList::new();
        {
            use protobuf;
            use std::iter::FromIterator;
            let set = self.uploads.lock().unwrap();
            let set = set.clone().into_iter().map(|x| {
                let mut val = grpc::backend::GenUuidResult::new();
                val.set_uuid(x.digest);
                val
            });
            resp.set_uuids(protobuf::RepeatedField::from_iter(set));
        }
        let f = sink.success(resp).map_err(
            move |e| warn!("failed to reply! {:?}", e),
        );
        ctx.spawn(f);
    }
}


#[cfg(test)]
mod test {
    // 1. start up a listening backend service
    // 2. test the exposed service
}
