use guest::prelude::*;
use k8s_openapi::api::{
    apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet},
    batch::v1::{CronJob, Job},
    core::v1::{Container, EphemeralContainer, Pod, PodSpec, ReplicationController},
};
use kubewarden_policy_sdk::wapc_guest as guest;
use lazy_static::lazy_static;
use serde::Serialize;

extern crate kubewarden_policy_sdk as kubewarden;
#[cfg(test)]
use crate::tests::mock_verification_sdk::{
    verify_certificate, verify_keyless_exact_match, verify_keyless_github_actions,
    verify_keyless_prefix_match, verify_pub_keys_image,
};
use anyhow::Result;
use kubewarden::host_capabilities::verification::VerificationResponse;
#[cfg(not(test))]
use kubewarden::host_capabilities::verification::{
    verify_certificate, verify_keyless_exact_match, verify_keyless_github_actions,
    verify_keyless_prefix_match, verify_pub_keys_image,
};
use kubewarden::{logging, protocol_version_guest, request::ValidationRequest, validate_settings};
use serde::de::DeserializeOwned;

mod settings;
use settings::Settings;

use crate::settings::Signature;
use slog::{o, warn, Logger};
use wildmatch::WildMatch;

lazy_static! {
    static ref LOG_DRAIN: Logger = Logger::root(
        logging::KubewardenDrain::new(),
        o!("policy" => "verify-image-signatures")
    );
}

#[no_mangle]
pub extern "C" fn wapc_init() {
    register_function("validate", validate);
    register_function("validate_settings", validate_settings::<Settings>);
    register_function("protocol_version", protocol_version_guest);
}

/// Represents an abstraction of an struct that contains an image
/// Used to reuse code for Container and EphemeralContainer
trait ImageHolder: Clone {
    fn set_image(&mut self, image: Option<String>);
    fn get_image(&self) -> Option<String>;
}

impl ImageHolder for Container {
    fn set_image(&mut self, image: Option<String>) {
        self.image = image;
    }

    fn get_image(&self) -> Option<String> {
        self.image.clone()
    }
}

impl ImageHolder for EphemeralContainer {
    fn set_image(&mut self, image: Option<String>) {
        self.image = image;
    }

    fn get_image(&self) -> Option<String> {
        self.image.clone()
    }
}

/// Represents all resources that can be validated with this policy
trait ValidatingResource {
    fn name(&self) -> String;
    fn spec(&self) -> Option<PodSpec>;
    fn set_spec(&mut self, spec: PodSpec);
}

impl ValidatingResource for Pod {
    fn name(&self) -> String {
        self.metadata.name.clone().unwrap_or_default()
    }

    fn spec(&self) -> Option<PodSpec> {
        self.spec.clone()
    }

    fn set_spec(&mut self, spec: PodSpec) {
        self.spec = Some(spec);
    }
}

impl ValidatingResource for Deployment {
    fn name(&self) -> String {
        self.metadata.name.clone().unwrap_or_default()
    }

    fn spec(&self) -> Option<PodSpec> {
        self.spec.as_ref()?.template.spec.clone()
    }

    fn set_spec(&mut self, spec: PodSpec) {
        self.spec.as_mut().unwrap().template.spec = Some(spec);
    }
}

impl ValidatingResource for ReplicaSet {
    fn name(&self) -> String {
        self.metadata.name.clone().unwrap_or_default()
    }

    fn spec(&self) -> Option<PodSpec> {
        self.spec.as_ref()?.template.as_ref()?.spec.clone()
    }

    fn set_spec(&mut self, spec: PodSpec) {
        self.spec.as_mut().unwrap().template.as_mut().unwrap().spec = Some(spec);
    }
}

impl ValidatingResource for StatefulSet {
    fn name(&self) -> String {
        self.metadata.name.clone().unwrap_or_default()
    }

    fn spec(&self) -> Option<PodSpec> {
        self.spec.as_ref()?.template.spec.clone()
    }

    fn set_spec(&mut self, spec: PodSpec) {
        self.spec.as_mut().unwrap().template.spec = Some(spec);
    }
}

impl ValidatingResource for DaemonSet {
    fn name(&self) -> String {
        self.metadata.name.clone().unwrap_or_default()
    }

    fn spec(&self) -> Option<PodSpec> {
        self.spec.as_ref()?.template.spec.clone()
    }

    fn set_spec(&mut self, spec: PodSpec) {
        self.spec.as_mut().unwrap().template.spec = Some(spec);
    }
}

impl ValidatingResource for ReplicationController {
    fn name(&self) -> String {
        self.metadata.name.clone().unwrap_or_default()
    }

    fn spec(&self) -> Option<PodSpec> {
        self.spec.as_ref()?.template.as_ref()?.spec.clone()
    }

    fn set_spec(&mut self, spec: PodSpec) {
        self.spec.as_mut().unwrap().template.as_mut().unwrap().spec = Some(spec);
    }
}

impl ValidatingResource for Job {
    fn name(&self) -> String {
        self.metadata.name.clone().unwrap_or_default()
    }

    fn spec(&self) -> Option<PodSpec> {
        self.spec.as_ref()?.template.spec.clone()
    }

    fn set_spec(&mut self, spec: PodSpec) {
        self.spec.as_mut().unwrap().template.spec = Some(spec);
    }
}

impl ValidatingResource for CronJob {
    fn name(&self) -> String {
        self.metadata.name.clone().unwrap_or_default()
    }

    fn spec(&self) -> Option<PodSpec> {
        self.spec
            .as_ref()?
            .job_template
            .spec
            .as_ref()?
            .template
            .spec
            .clone()
    }

    fn set_spec(&mut self, spec: PodSpec) {
        self.spec
            .as_mut()
            .unwrap()
            .job_template
            .spec
            .as_mut()
            .unwrap()
            .template
            .spec = Some(spec);
    }
}

fn validate(payload: &[u8]) -> CallResult {
    let validation_request: ValidationRequest<Settings> = ValidationRequest::new(payload)?;

    match validation_request.request.kind.kind.as_str() {
        "Deployment" => validate_resource::<Deployment>(validation_request),
        "ReplicaSet" => validate_resource::<ReplicaSet>(validation_request),
        "StatefulSet" => validate_resource::<StatefulSet>(validation_request),
        "DaemonSet" => validate_resource::<DaemonSet>(validation_request),
        "ReplicationController" => validate_resource::<ReplicationController>(validation_request),
        "Job" => validate_resource::<Job>(validation_request),
        "CronJob" => validate_resource::<CronJob>(validation_request),
        "Pod" => validate_resource::<Pod>(validation_request),
        _ => {
            // We were forwarded a request we cannot unmarshal or
            // understand, just accept it
            warn!(LOG_DRAIN, "cannot unmarshal resource: this policy does not know how to evaluate this resource; accept it");
            kubewarden::accept_request()
        }
    }
}

// validate any resource that contains a Pod. e.g. Deployment, StatefulSet, ...
// it does not modify the container with the manifest digest.
fn validate_resource<T: ValidatingResource + DeserializeOwned + Serialize>(
    validation_request: ValidationRequest<Settings>,
) -> CallResult {
    let resource = match serde_json::from_value::<T>(validation_request.request.object.clone()) {
        Ok(resource) => resource,
        Err(_) => {
            // We were forwarded a request we cannot unmarshal or
            // understand, just accept it
            warn!(LOG_DRAIN, "cannot unmarshal resource: this policy does not know how to evaluate this resource; accept it");
            return kubewarden::accept_request();
        }
    };

    let spec = match resource.spec() {
        Some(spec) => spec,
        None => {
            return kubewarden::accept_request();
        }
    };

    let changed_spec =
        match verify_all_images_in_pod(&spec, &validation_request.settings.signatures) {
            Ok(spec) => match spec {
                Some(spec) => spec,
                None => {
                    return kubewarden::accept_request();
                }
            },
            Err(error) => {
                return kubewarden::reject_request(
                    Some(format!(
                        "Resource {} is not accepted: {}",
                        &resource.name(),
                        error
                    )),
                    None,
                    None,
                    None,
                );
            }
        };

    if !validation_request.settings.modify_images_with_digest {
        return kubewarden::accept_request();
    }

    let mut resource = resource;
    resource.set_spec(changed_spec);

    let mutated_object = serde_json::to_value(&resource)?;
    kubewarden::mutate_request(mutated_object)
}

/// verify all images and return a PodSpec with the images replaced with the digest which was used for the verification
fn verify_all_images_in_pod(
    spec: &PodSpec,
    signatures: &[Signature],
) -> Result<Option<PodSpec>, String> {
    let mut policy_verification_errors: Vec<String> = vec![];
    let mut spec_images_with_digest = spec.clone();
    let mut is_modified_with_digest = false;

    if let Some(containers_with_digest) = verify_container_images(
        &spec.containers,
        &mut policy_verification_errors,
        signatures,
    ) {
        spec_images_with_digest.containers = containers_with_digest;
        is_modified_with_digest = true;
    }
    if let Some(init_containers) = &spec.init_containers {
        if let Some(init_containers_with_digest) =
            verify_container_images(init_containers, &mut policy_verification_errors, signatures)
        {
            spec_images_with_digest.init_containers = Some(init_containers_with_digest);
            is_modified_with_digest = true;
        }
    }
    if let Some(ephemeral_containers) = &spec.ephemeral_containers {
        if let Some(ephemeral_containers_with_digest) = verify_container_images(
            ephemeral_containers,
            &mut policy_verification_errors,
            signatures,
        ) {
            spec_images_with_digest.ephemeral_containers = Some(ephemeral_containers_with_digest);
            is_modified_with_digest = true;
        }
    }

    if !policy_verification_errors.is_empty() {
        return Err(policy_verification_errors.join(", "));
    }

    if is_modified_with_digest {
        Ok(Some(spec_images_with_digest))
    } else {
        Ok(None)
    }
}

// verify images and return containers with the images replaced with the digest which was used for the verification
fn verify_container_images<T>(
    containers: &[T],
    policy_verification_errors: &mut Vec<String>,
    signatures: &[Signature],
) -> Option<Vec<T>>
where
    T: ImageHolder + PartialEq,
{
    let mut container_with_images_digests = containers.to_owned();

    for (i, container) in containers.iter().enumerate() {
        let container_image = container.get_image().unwrap();

        for signature in signatures.iter() {
            // verify if the name matches the image name provided
            if !WildMatch::new(signature.image()).matches(container_image.as_str()) {
                continue;
            }

            let verification_response = match signature {
                Signature::PubKeys(s) => verify_pub_keys_image(
                    container_image.as_str(),
                    s.pub_keys.clone(),
                    s.annotations.clone(),
                ),
                Signature::Keyless(s) => verify_keyless_exact_match(
                    container_image.as_str(),
                    s.keyless.clone(),
                    s.annotations.clone(),
                ),
                Signature::KeylessPrefix(s) => verify_keyless_prefix_match(
                    container_image.as_str(),
                    s.keyless_prefix.clone(),
                    s.annotations.clone(),
                ),
                Signature::GithubActions(s) => verify_keyless_github_actions(
                    container_image.as_str(),
                    s.github_actions.owner.clone(),
                    s.github_actions.repo.clone(),
                    s.annotations.clone(),
                ),
                Signature::Certificate(s) => {
                    let mut response: Result<VerificationResponse> =
                        Err(anyhow::anyhow!("Cannot verify"));

                    for (index, certificate) in s.certificates.iter().enumerate() {
                        response = verify_certificate(
                            container_image.as_str(),
                            certificate.clone(),
                            s.certificate_chain.clone(),
                            s.require_rekor_bundle,
                            s.annotations.clone(),
                        );
                        // All the certificates must be verified. As soon as one of
                        // them cannot be used to verify the image -> break from the
                        // loop and propagate the verification failure
                        if response.is_err() {
                            warn!(
                                LOG_DRAIN,
                                "certificate image verification failed";
                                "image" => container_image.clone(),
                                "certificate-index" => index,
                            );
                            break;
                        }
                    }
                    response
                }
            };

            handle_verification_response(
                verification_response,
                container_image.as_str(),
                &mut container_with_images_digests[i],
                policy_verification_errors,
            );
        }
    }

    if containers != container_with_images_digests {
        Some(container_with_images_digests.to_vec())
    } else {
        None
    }
}

fn handle_verification_response<T>(
    response: Result<VerificationResponse>,
    container_image: &str,
    container_with_images_digests: &mut T,
    policy_verification_errors: &mut Vec<String>,
) where
    T: ImageHolder,
{
    match response {
        Ok(response) => add_digest_if_not_present(
            container_image,
            response.digest.as_str(),
            container_with_images_digests,
        ),
        Err(e) => {
            policy_verification_errors.push(format!(
                "verification of image {container_image} failed: {e}"
            ));
        }
    };
}

// returns true if digest was appended
fn add_digest_if_not_present<T>(
    container_image: &str,
    digest: &str,
    container_with_images_digests: &mut T,
) where
    T: ImageHolder,
{
    if !container_image.contains(digest) {
        let image_with_digest = [container_image, digest].join("@");
        container_with_images_digests.set_image(Some(image_with_digest));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        github_actions::KeylessGithubActionsInfo, Certificate, GithubActions, Keyless,
        KeylessPrefix, PubKeys,
    };
    use anyhow::anyhow;
    use kubewarden::{
        host_capabilities::verification::{KeylessInfo, KeylessPrefixInfo, VerificationResponse},
        request::{GroupVersionKind, KubernetesAdmissionRequest},
        response::ValidationResponse,
        test::Testcase,
    };
    use mockall::automock;
    use rstest::*;
    use serde_json::json;
    use serial_test::serial;

    #[automock()]
    pub mod crypto_sdk {
        use anyhow::Result;
        use kubewarden::host_capabilities::crypto::{BoolWithReason, Certificate};

        #[allow(dead_code)]
        pub fn verify_cert(
            _cert: Certificate,
            _cert_chain: Option<Vec<Certificate>>,
            _not_after: Option<String>,
        ) -> Result<BoolWithReason> {
            Ok(BoolWithReason::True)
        }
    }

    #[automock()]
    pub mod verification_sdk {
        use anyhow::Result;
        use kubewarden::host_capabilities::verification::{
            KeylessInfo, KeylessPrefixInfo, VerificationResponse,
        };
        use std::collections::BTreeMap;

        // needed for creating mocks
        #[allow(dead_code)]
        pub fn verify_pub_keys_image(
            _image: &str,
            _pub_keys: Vec<String>,
            _annotations: Option<BTreeMap<String, String>>,
        ) -> Result<VerificationResponse> {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "mock_digest".to_string(),
            })
        }

        // needed for creating mocks
        #[allow(dead_code)]
        pub fn verify_keyless_exact_match(
            _image: &str,
            _keyless: Vec<KeylessInfo>,
            _annotations: Option<BTreeMap<String, String>>,
        ) -> Result<VerificationResponse> {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "mock_digest".to_string(),
            })
        }

        // needed for creating mocks
        #[allow(dead_code)]
        pub fn verify_keyless_prefix_match(
            _image: &str,
            _keyless_prefix: Vec<KeylessPrefixInfo>,
            _annotations: Option<BTreeMap<String, String>>,
        ) -> Result<VerificationResponse> {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "mock_digest".to_string(),
            })
        }

        // needed for creating mocks
        #[allow(dead_code)]
        pub fn verify_keyless_github_actions(
            _image: &str,
            _owner: String,
            _repo: Option<String>,
            _annotations: Option<BTreeMap<String, String>>,
        ) -> Result<VerificationResponse> {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "mock_digest".to_string(),
            })
        }

        // needed for creating mocks
        #[allow(dead_code)]
        pub fn verify_certificate(
            _image: &str,
            _certificate: String,
            _certificate_chain: Option<Vec<String>>,
            _require_rekor_bundle: bool,
            _annotations: Option<BTreeMap<String, String>>,
        ) -> Result<VerificationResponse> {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "mock_digest".to_string(),
            })
        }
    }

    fn image_url(has_digest: bool) -> &'static str {
        if has_digest {
            "ghcr.io/kubewarden/test-verify-image-signatures:signed@sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
        } else {
            "ghcr.io/kubewarden/test-verify-image-signatures:signed"
        }
    }

    fn pod(has_digest: bool) -> serde_json::Value {
        json!(
        {
          "apiVersion": "v1",
          "kind": "Pod",
          "metadata": {
            "name": "nginx"
          },
          "spec": {
            "containers": [
              {
                "image": image_url(has_digest),
                "name": "test-verify-image-signatures"
              }
            ]
          }
        })
    }

    fn deployment(has_digest: bool) -> serde_json::Value {
        json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": "nginx-deployment"
            },
            "spec": {
                "replicas": 3,
                "selector": {
                    "matchLabels": {
                        "app": "nginx"
                    }
                },
                "template": {
                    "metadata": {
                        "labels": {
                            "app": "nginx"
                        }
                    },
                    "spec": {
                        "containers": [
                            {
                                "image": image_url(has_digest),
                                "name": "test-verify-image-signatures"
                            }
                        ]
                    }
                }
            }
        })
    }

    fn replica_set(has_digest: bool) -> serde_json::Value {
        json!(
        {
          "apiVersion": "apps/v1",
          "kind": "ReplicaSet",
          "metadata": {
            "name": "nginx"
          },
          "spec": {
            "replicas": 3,
            "selector": {
              "matchLabels": {
                "app": "nginx"
              }
            },
            "template": {
              "metadata": {
                "labels": {
                  "app": "nginx"
                }
              },
              "spec": {
                "containers": [
                  {
                    "image": image_url(has_digest),
                    "name": "test-verify-image-signatures"
                  }
                ]
              }
            }
          }
        })
    }

    fn daemon_set(has_digest: bool) -> serde_json::Value {
        json!(
        {
          "apiVersion": "apps/v1",
          "kind": "DaemonSet",
          "metadata": {
            "name": "nginx"
          },
          "spec": {
            "selector": {
              "matchLabels": {
                "app": "nginx"
              }
            },
            "template": {
              "metadata": {
                "labels": {
                  "app": "nginx"
                }
              },
              "spec": {
                "containers": [
                  {
                    "image": image_url(has_digest),
                    "name": "test-verify-image-signatures"
                  }
                ]
              }
            }
          }
        })
    }

    fn replication_controller(has_digest: bool) -> serde_json::Value {
        json!(
        {
          "apiVersion": "v1",
          "kind": "ReplicationController",
          "metadata": {
            "name": "nginx"
          },
          "spec": {
            "replicas": 3,
            "selector": {
              "app": "nginx"
            },
            "template": {
              "metadata": {
                "labels": {
                  "app": "nginx"
                }
              },
              "spec": {
                "containers": [
                  {
                    "image": image_url(has_digest),
                    "name": "test-verify-image-signatures"
                  }
                ]
              }
            }
          }
        })
    }

    fn job(has_digest: bool) -> serde_json::Value {
        json!(
        {
          "apiVersion": "batch/v1",
          "kind": "Job",
          "metadata": {
            "name": "nginx"
          },
          "spec": {
            "template": {
              "metadata": {
                "labels": {
                  "app": "nginx"
                }
              },
              "spec": {
                "containers": [
                  {
                    "image": image_url(has_digest),
                    "name": "test-verify-image-signatures"
                  }
                ],
                "restartPolicy": "Never"
              }
            }
          }
        })
    }

    fn cron_job(has_digest: bool) -> serde_json::Value {
        json!(
        {
          "apiVersion": "batch/v1",
          "kind": "CronJob",
          "metadata": {
            "name": "nginx"
          },
          "spec": {
            "schedule": "*/1 * * * *",
            "jobTemplate": {
              "spec": {
                "template": {
                  "metadata": {
                    "labels": {
                      "app": "nginx"
                    }
                  },
                  "spec": {
                    "containers": [
                      {
                        "image": image_url(has_digest),
                        "name": "test-verify-image-signatures"
                      }
                    ],
                    "restartPolicy": "OnFailure"
                  }
                }
              }
            }
          }
        })
    }

    // these tests need to run sequentially because mockall creates a global context to create the mocks
    #[rstest]
    #[case::pod(pod(false), pod(true))]
    #[case::deployment(deployment(false), deployment(true))]
    #[case::replica_set(replica_set(false), replica_set(true))]
    #[case::daemon_set(daemon_set(false), daemon_set(true))]
    #[case::replication_controller(replication_controller(false), replication_controller(true))]
    #[case::job(job(false), job(true))]
    #[case::cron_job(cron_job(false), cron_job(true))]
    #[serial] // these tests need to run sequentially because mockall creates a global context to create the mocks
    fn mutation(#[case] resource: serde_json::Value, #[case] expected_mutation: serde_json::Value) {
        let ctx = mock_verification_sdk::verify_pub_keys_image_context();
        ctx.expect().times(2).returning(|_, _, _| {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
                    .to_string(),
            })
        });

        for allow_mutation in [true, false] {
            let resource = resource.clone();

            let settings: Settings = Settings {
                signatures: vec![Signature::PubKeys(PubKeys {
                    image: "ghcr.io/kubewarden/test-verify-image-signatures:*".to_string(),
                    pub_keys: vec!["key".to_string()],
                    annotations: None,
                })],
                modify_images_with_digest: allow_mutation,
            };

            let request = ValidationRequest {
                request: KubernetesAdmissionRequest {
                    kind: GroupVersionKind {
                        kind: resource["kind"].as_str().unwrap().to_string(),
                        ..Default::default()
                    },
                    object: resource,
                    ..Default::default()
                },
                settings,
            };

            let response = validate(serde_json::to_vec(&request).unwrap().as_slice()).unwrap();
            let response: ValidationResponse = serde_json::from_slice(&response).unwrap();
            assert!(response.accepted);

            if allow_mutation {
                assert_eq!(response.mutated_object.unwrap(), expected_mutation);
            } else {
                assert!(response.mutated_object.is_none());
            }
        }
    }

    #[test]
    #[serial]
    fn pub_keys_validation_dont_pass() {
        let ctx = mock_verification_sdk::verify_pub_keys_image_context();
        ctx.expect()
            .times(1)
            .returning(|_, _, _| Err(anyhow!("error")));

        let settings: Settings = Settings {
            signatures: vec![Signature::PubKeys(PubKeys {
                image: "*".to_string(),
                pub_keys: vec!["key".to_string()],
                annotations: None,
            })],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should fail when validating the nginx container"),
            fixture_file: String::from("test_data/pod_creation_signed.json"),
            settings,
            expected_validation_result: false,
        };

        let response = tc.eval(validate).unwrap();
        assert!(!response.accepted);
        assert!(response.mutated_object.is_none());
    }

    #[test]
    #[serial]
    fn keyless_validation_pass_with_mutation() {
        let ctx = mock_verification_sdk::verify_keyless_exact_match_context();
        ctx.expect().times(1).returning(|_, _, _| {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
                    .to_string(),
            })
        });

        let settings: Settings = Settings {
            signatures: vec![Signature::Keyless(Keyless {
                image: "ghcr.io/kubewarden/test-verify-image-signatures:*".to_string(),
                keyless: vec![KeylessInfo {
                    issuer: "issuer".to_string(),
                    subject: "subject".to_string(),
                }],
                annotations: None,
            })],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should successfully validate the nginx container"),
            fixture_file: String::from("test_data/pod_creation_signed.json"),
            settings,
            expected_validation_result: true,
        };

        let response = tc.eval(validate).unwrap();
        assert!(response.accepted);
        let expected_mutation: serde_json::Value = json!(
        {
          "apiVersion": "v1",
          "kind": "Pod",
          "metadata": {
            "name": "nginx"
          },
          "spec": {
            "containers": [
              {
                "image": "ghcr.io/kubewarden/test-verify-image-signatures:signed@sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e",
                "name": "test-verify-image-signatures"
              }
            ]
          }
        });
        assert_eq!(response.mutated_object.unwrap(), expected_mutation);
    }

    #[test]
    #[serial]
    fn keyless_validation_dont_pass() {
        let ctx = mock_verification_sdk::verify_keyless_exact_match_context();
        ctx.expect()
            .times(1)
            .returning(|_, _, _| Err(anyhow!("error")));

        let settings: Settings = Settings {
            signatures: vec![Signature::Keyless(Keyless {
                image: "ghcr.io/kubewarden/test-verify-image-signatures:*".to_string(),
                keyless: vec![],
                annotations: None,
            })],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should fail when validating the ghcr.io/kubewarden/test-verify-image-signatures container"),
            fixture_file: String::from("test_data/pod_creation_signed.json"),
            settings,
            expected_validation_result: false,
        };

        let response = tc.eval(validate).unwrap();
        assert!(!response.accepted)
    }

    #[test]
    #[serial]
    fn certificate_validation_pass_with_no_mutation() {
        let ctx = mock_verification_sdk::verify_certificate_context();
        ctx.expect()
            .times(1)
            .returning(|_, certificate, _, _, _| match certificate.as_str() {
                "good-cert" => Ok(VerificationResponse {
                    is_trusted: true,
                    digest:
                        "sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
                            .to_string(),
                }),
                _ => Err(anyhow!("not good-cert")),
            });

        let settings: Settings = Settings {
            signatures: vec![Signature::Certificate(Certificate {
                image: "ghcr.io/kubewarden/test-verify-image-signatures:*".to_string(),
                certificates: vec!["good-cert".to_string()],
                certificate_chain: None,
                require_rekor_bundle: true,
                annotations: None,
            })],
            modify_images_with_digest: false,
        };

        let tc = Testcase {
            name: String::from("It should successfully validate the ghcr.io/kubewarden/test-verify-image-signatures container"),
            fixture_file: String::from("test_data/pod_creation_signed.json"),
            settings,
            expected_validation_result: true,
        };

        let response = tc.eval(validate).unwrap();
        assert!(response.accepted);
        assert!(response.mutated_object.is_none());
    }

    #[test]
    #[serial]
    fn certificate_validation_pass_with_multiple_good_keys() {
        let ctx = mock_verification_sdk::verify_certificate_context();
        ctx.expect()
            .times(2)
            .returning(|_, certificate, _, _, _| match certificate.as_str() {
                "good-cert1" | "good-cert2" => Ok(VerificationResponse {
                    is_trusted: true,
                    digest:
                        "sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
                            .to_string(),
                }),
                _ => Err(anyhow!("not good-cert")),
            });

        let settings: Settings = Settings {
            signatures: vec![Signature::Certificate(Certificate {
                image: "ghcr.io/kubewarden/test-verify-image-signatures:*".to_string(),
                certificates: vec!["good-cert1".to_string(), "good-cert2".to_string()],
                certificate_chain: None,
                require_rekor_bundle: true,
                annotations: None,
            })],
            modify_images_with_digest: false,
        };

        let tc = Testcase {
            name: String::from("It should successfully validate the ghcr.io/kubewarden/test-verify-image-signatures container"),
            fixture_file: String::from("test_data/pod_creation_signed.json"),
            settings,
            expected_validation_result: true,
        };

        let response = tc.eval(validate).unwrap();
        assert!(response.accepted);
        assert!(response.mutated_object.is_none());
    }

    #[test]
    #[serial]
    fn certificate_validation_dont_pass() {
        let ctx = mock_verification_sdk::verify_certificate_context();
        ctx.expect()
            .times(2)
            .returning(|_, certificate, _, _, _| match certificate.as_str() {
                "good-cert" => Ok(VerificationResponse {
                    is_trusted: true,
                    digest:
                        "sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
                            .to_string(),
                }),
                _ => Err(anyhow!("not good-cert")),
            });

        // validation with 2 certs, one of the is good the other isn't
        let settings: Settings = Settings {
            signatures: vec![Signature::Certificate(Certificate {
                image: "ghcr.io/kubewarden/test-verify-image-signatures:*".to_string(),
                certificates: vec!["good-cert".to_string(), "bad-cert".to_string()],
                certificate_chain: None,
                require_rekor_bundle: true,
                annotations: None,
            })],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should fail when validating the nginx container"),
            fixture_file: String::from("test_data/pod_creation_signed.json"),
            settings,
            expected_validation_result: false,
        };

        let response = tc.eval(validate).unwrap();
        assert!(!response.accepted);
        assert!(response.mutated_object.is_none());
    }

    #[test]
    #[serial]
    fn validation_pass_when_there_is_no_matching_containers() {
        let ctx = mock_verification_sdk::verify_pub_keys_image_context();
        ctx.expect()
            .times(0)
            .returning(|_, _, _| Err(anyhow!("error")));

        let ctx = mock_verification_sdk::verify_keyless_exact_match_context();
        ctx.expect()
            .times(0)
            .returning(|_, _, _| Err(anyhow!("error")));

        let settings: Settings = Settings {
            signatures: vec![
                Signature::PubKeys(PubKeys {
                    image: "no_matching".to_string(),
                    pub_keys: vec![],
                    annotations: None,
                }),
                Signature::Keyless(Keyless {
                    image: "no_matching".to_string(),
                    keyless: vec![],
                    annotations: None,
                }),
            ],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should return true since there is no matching containers"),
            fixture_file: String::from("test_data/pod_creation_signed.json"),
            settings,
            expected_validation_result: true,
        };

        let response = tc.eval(validate).unwrap();
        assert!(response.accepted);
        assert!(response.mutated_object.is_none());
    }

    #[test]
    #[serial]
    fn validation_with_multiple_containers_fail_if_one_fails() {
        let ctx_pub_keys = mock_verification_sdk::verify_pub_keys_image_context();
        ctx_pub_keys.expect().times(1).returning(|_, _, _| {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
                    .to_string(),
            })
        });

        let ctx_keyless = mock_verification_sdk::verify_keyless_exact_match_context();
        ctx_keyless
            .expect()
            .times(1)
            .returning(|_, _, _| Err(anyhow!("error")));

        let settings: Settings = Settings {
            signatures: vec![
                Signature::Keyless(Keyless {
                    image: "nginx".to_string(),
                    keyless: vec![KeylessInfo {
                        issuer: "issuer".to_string(),
                        subject: "subject".to_string(),
                    }],
                    annotations: None,
                }),
                Signature::PubKeys(PubKeys {
                    image: "init".to_string(),
                    pub_keys: vec![],
                    annotations: None,
                }),
            ],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should fail because one validation fails"),
            fixture_file: String::from("test_data/pod_creation_with_init_container.json"),
            settings,
            expected_validation_result: false,
        };

        let response = tc.eval(validate).unwrap();
        assert!(!response.accepted);
        assert!(response.mutated_object.is_none());
    }

    #[test]
    #[serial]
    fn validation_with_multiple_containers_with_mutation_pass() {
        let ctx_pub_keys = mock_verification_sdk::verify_pub_keys_image_context();
        ctx_pub_keys.expect().times(1).returning(|_, _, _| {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
                    .to_string(),
            })
        });

        let ctx_keyless = mock_verification_sdk::verify_keyless_exact_match_context();
        ctx_keyless.expect().times(1).returning(|_, _, _| {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "sha256:a3d850c2022ebf02156114178ef35298d63f83c740e7b5dd7777ff05898880f8"
                    .to_string(),
            })
        });

        let settings: Settings = Settings {
            signatures: vec![
                Signature::Keyless(Keyless {
                    image: "nginx".to_string(),
                    keyless: vec![KeylessInfo {
                        issuer: "issuer".to_string(),
                        subject: "subject".to_string(),
                    }],
                    annotations: None,
                }),
                Signature::PubKeys(PubKeys {
                    image: "init".to_string(),
                    pub_keys: vec![],
                    annotations: None,
                }),
            ],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should successfully validate the nginx and init containers"),
            fixture_file: String::from("test_data/pod_creation_with_init_container.json"),
            settings,
            expected_validation_result: true,
        };

        let response = tc.eval(validate).unwrap();
        assert!(response.accepted);

        let expected: serde_json::Value = json!(
                    {
          "apiVersion": "v1",
          "kind": "Pod",
          "metadata": {
            "name": "nginx"
          },
          "spec": {
            "containers": [
              {
                "image": "nginx@sha256:a3d850c2022ebf02156114178ef35298d63f83c740e7b5dd7777ff05898880f8",
                "name": "nginx"
              }
            ],
            "initContainers": [
              {
                "image": "init@sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e",
                "name": "init"
              }
            ]
          }
        }
                );
        assert_eq!(response.mutated_object.unwrap(), expected);
    }

    #[test]
    #[serial]
    fn keyless_validation_pass_and_dont_mutate_if_digest_is_present() {
        let ctx = mock_verification_sdk::verify_keyless_exact_match_context();
        ctx.expect().times(1).returning(|_, _, _| {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
                    .to_string(),
            })
        });

        let settings: Settings = Settings {
            signatures: vec![Signature::Keyless(Keyless {
                image: "nginx:*".to_string(),
                keyless: vec![KeylessInfo {
                    issuer: "issuer".to_string(),
                    subject: "subject".to_string(),
                }],
                annotations: None,
            })],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should successfully validate the nginx container"),
            fixture_file: String::from("test_data/pod_creation_with_digest.json"),
            settings,
            expected_validation_result: true,
        };

        let response = tc.eval(validate).unwrap();
        assert!(response.accepted);
        assert!(response.mutated_object.is_none())
    }

    #[test]
    #[serial]
    fn keyless_prefix_validation_pass_and_dont_mutate_if_digest_is_present() {
        let ctx = mock_verification_sdk::verify_keyless_prefix_match_context();
        ctx.expect().times(1).returning(|_, _, _| {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
                    .to_string(),
            })
        });

        let settings: Settings = Settings {
            signatures: vec![Signature::KeylessPrefix(KeylessPrefix {
                image: "nginx:*".to_string(),
                keyless_prefix: vec![KeylessPrefixInfo {
                    issuer: "issuer".to_string(),
                    url_prefix: "subject".to_string(),
                }],
                annotations: None,
            })],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should successfully validate the nginx container"),
            fixture_file: String::from("test_data/pod_creation_with_digest.json"),
            settings,
            expected_validation_result: true,
        };

        let response = tc.eval(validate).unwrap();
        assert!(response.accepted);
        assert!(response.mutated_object.is_none())
    }

    #[test]
    #[serial]
    fn keyless_github_action_validation_pass_and_dont_mutate_if_digest_is_present() {
        let ctx = mock_verification_sdk::verify_keyless_github_actions_context();
        ctx.expect().times(1).returning(|_, _, _, _| {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "sha256:89102e348749bb17a6a651a4b2a17420e1a66d2a44a675b981973d49a5af3a5e"
                    .to_string(),
            })
        });

        let settings: Settings = Settings {
            signatures: vec![Signature::GithubActions(GithubActions {
                image: "nginx:*".to_string(),
                github_actions: KeylessGithubActionsInfo {
                    owner: "owner".to_string(),
                    repo: Some("repo".to_string()),
                },
                annotations: None,
            })],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should successfully validate the nginx container"),
            fixture_file: String::from("test_data/pod_creation_with_digest.json"),
            settings,
            expected_validation_result: true,
        };

        let response = tc.eval(validate).unwrap();
        assert!(response.accepted);
        assert!(response.mutated_object.is_none())
    }

    fn resource_validation_pass(file: &str) {
        let ctx = mock_verification_sdk::verify_keyless_exact_match_context();
        ctx.expect().times(1).returning(|_, _, _| {
            Ok(VerificationResponse {
                is_trusted: true,
                digest: "".to_string(),
            })
        });

        let settings: Settings = Settings {
            signatures: vec![Signature::Keyless(Keyless {
                image: "*".to_string(),
                keyless: vec![KeylessInfo {
                    issuer: "issuer".to_string(),
                    subject: "subject".to_string(),
                }],
                annotations: None,
            })],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should successfully validate the nginx container"),
            fixture_file: String::from(file),
            settings,
            expected_validation_result: true,
        };

        let response = tc.eval(validate).unwrap();
        assert!(response.accepted);
        assert!(response.mutated_object.is_none())
    }

    fn resource_validation_reject(file: &str) {
        let ctx = mock_verification_sdk::verify_keyless_exact_match_context();
        ctx.expect()
            .times(1)
            .returning(|_, _, _| Err(anyhow!("error")));

        let settings: Settings = Settings {
            signatures: vec![Signature::Keyless(Keyless {
                image: "*".to_string(),
                keyless: vec![KeylessInfo {
                    issuer: "issuer".to_string(),
                    subject: "subject".to_string(),
                }],
                annotations: None,
            })],
            modify_images_with_digest: true,
        };

        let tc = Testcase {
            name: String::from("It should failed validation"),
            fixture_file: String::from(file),
            settings,
            expected_validation_result: false,
        };

        let response = tc.eval(validate).unwrap();
        assert!(!response.accepted);
        assert!(response.mutated_object.is_none())
    }

    #[test]
    #[serial]
    fn resources_validation() {
        resource_validation_pass("test_data/deployment_creation_signed.json");
        resource_validation_pass("test_data/statefulset_creation_signed.json");
        resource_validation_pass("test_data/daemonset_creation_signed.json");
        resource_validation_pass("test_data/replicaset_creation_signed.json");
        resource_validation_pass("test_data/replicationcontroller_creation_signed.json");
        resource_validation_pass("test_data/cronjob_creation_signed.json");
        resource_validation_pass("test_data/job_creation_signed.json");

        resource_validation_reject("test_data/deployment_creation_unsigned.json");
        resource_validation_reject("test_data/statefulset_creation_unsigned.json");
        resource_validation_reject("test_data/daemonset_creation_unsigned.json");
        resource_validation_reject("test_data/replicaset_creation_unsigned.json");
        resource_validation_reject("test_data/replicationcontroller_creation_unsigned.json");
        resource_validation_reject("test_data/cronjob_creation_unsigned.json");
        resource_validation_reject("test_data/job_creation_unsigned.json");
    }
}
