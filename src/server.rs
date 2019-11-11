use std::io::Read;
use std::sync::Arc;

use byte_unit::Byte;
use bytes::Buf;
use jsonwebtoken::{decode, Validation};
use snafu::{ResultExt, Snafu};
use warp::{self, filters, http::StatusCode, path, Filter, Rejection, Reply};

use crate::configuration::*;
use crate::kvstore::KvStore;
use crate::logger::{LogLevel, Logger};

#[derive(Serialize, Deserialize)]
struct JsonMessage {
    message: String,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    iss: String,
    iat: i64,
    exp: i64,
}

pub struct Server {
    configuration: Configuration,
}

impl Server {
    pub fn new() -> Server {
        Server {
            configuration: Configuration::default(),
        }
    }

    pub fn configure(&mut self, configuration: Configuration) {
        self.configuration = configuration;
    }

    pub fn run(&self) {
        let store = Arc::new(KvStore::new());
        let store = warp::any().map(move || store.clone());

        let api_kv = path!("api" / "kv").and(path::end()).and(
            warp::get2()
                .and(store.clone())
                .and(warp::query::<GetKeyParameters>())
                .and_then(get_key)
                .or(warp::put2()
                    .and(store.clone())
                    .and(warp::query::<PutKeyParameters>())
                    .and(filters::body::content_length_limit(self.configuration.store.max_limit))
                    .and(warp::body::concat())
                    .and_then(put_key)),
        );
        let routes = api_kv.recover(process_error);
        warp::serve(routes).run(([127, 0, 0, 1], 7021));
    }
}

#[derive(Debug, Deserialize)]
struct GetKeyParameters {
    key: Option<String>,
}
fn get_key(store: Arc<KvStore>, parameters: GetKeyParameters) -> Result<impl Reply, Rejection> {
    if let Some(key) = parameters.key {
        if let Some(value) = store.get(key) {
            Ok(value)
        } else {
            Err(warp::reject::custom(Error::KeyNotFound))
        }
    } else {
        Err(warp::reject::custom(Error::MissingParameter {
            name: "key".to_string(),
        }))
    }
}

#[derive(Debug, Deserialize)]
struct PutKeyParameters {
    key: Option<String>,
}
fn put_key(
    store: Arc<KvStore>,
    parameters: PutKeyParameters,
    body: filters::body::FullBody,
) -> Result<impl Reply, Rejection> {
    if body.remaining() == 0 {
        Err(warp::reject::custom(Error::MissingBody))
    } else {
        if let Some(key) = parameters.key {
            if let Some(_) = store.set(key, body.bytes().to_vec()) {
                Ok(warp::reply::json(&JsonMessage {
                    message: "The specified key was successfully updated.".to_string(),
                }))
            } else {
                Ok(warp::reply::json(&JsonMessage {
                    message: "The specified key was successfully created.".to_string(),
                }))
            }
        } else {
            Err(warp::reject::custom(Error::MissingParameter {
                name: "key".to_string(),
            }))
        }
    }
}

fn process_error(err: Rejection) -> Result<impl Reply, Rejection> {
    if let Some(err) = err.find_cause::<Error>() {
        let code = match err {
            Error::MissingBody => StatusCode::BAD_REQUEST,
            Error::MissingParameter { .. } => StatusCode::BAD_REQUEST,
            Error::KeyNotFound => StatusCode::NOT_FOUND,
        };
        let json = warp::reply::json(&JsonMessage {
            message: err.to_string(),
        });
        Ok(warp::reply::with_status(json, code))
    } else if let Some(_) = err.find_cause::<warp::reject::MethodNotAllowed>() {
        let code = StatusCode::METHOD_NOT_ALLOWED;
        let json = warp::reply::json(&JsonMessage {
            message: "Method not allowed.".to_string(),
        });
        Ok(warp::reply::with_status(json, code))
    } else if let Some(_) = err.find_cause::<warp::reject::PayloadTooLarge>() {
        let code = StatusCode::METHOD_NOT_ALLOWED;
        let json = warp::reply::json(&JsonMessage {
            message: "Request payload is over {} bytes long.".to_string(), // TODO: format the string
        });
        Ok(warp::reply::with_status(json, code))
    } else {
        Err(err)
    }
}

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(display("Missing request body."))]
    MissingBody,
    #[snafu(display("Missing \"{}\" parameter.", name))]
    MissingParameter { name: String },
    #[snafu(display("The specified key does not exist."))]
    KeyNotFound,
}
