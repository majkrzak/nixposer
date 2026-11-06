use std::collections::BTreeMap;

use itertools::Itertools;
use nix_derivation::StorePath;
use reqwest::Client;
use thiserror::Error;
use url::Url;

use crate::{
    model::{
        ComposerAutoloadEntry, ComposerDependency, ComposerPackageIdentifier,
        ComposerPackageMetadata,
    },
    tree::{Node, Tree},
};

pub(super) mod wire {
    use serde_json::Value;
    use url::Url;

    use crate::model::ComposerPackageIdentifier;

    typify::import_types!(
        schema = "./schema/composer-repository-schema.json",
        map_type = ::std::collections::BTreeMap,
    );

    impl ComposerPackageRepository {
        pub fn package<'a>(
            &'a self,
            identifier: &ComposerPackageIdentifier,
        ) -> Option<&'a Package> {
            if let Some(Packages::Object(packages)) = self.packages.as_ref() {
                match packages.get(&identifier.name().to_string())? {
                    PackageVersions::Object(versions) => {
                        versions.get(&identifier.version().to_string())
                    }
                    PackageVersions::Array(versions) => versions.iter().find(|package| {
                        package
                            .version
                            .as_ref()
                            .is_some_and(|v| *v == identifier.version().to_string())
                    }),
                }
            } else {
                None
            }
        }
    }

    impl Package {
        pub fn download_url(&self) -> Option<Url> {
            Url::parse(self.dist.as_ref()?.url.as_ref()?).ok()
        }
    }

    // Reimplementation of `Composer\Util\MetadataMinifier::expand()`
    pub fn expand(mut root: Value) -> Value {
        if root.get("minified").and_then(Value::as_str) != Some("composer/2.0") {
            return root;
        }
        let Some(repo) = root.as_object_mut() else {
            return root;
        };

        let Some(packages) = repo.get_mut("packages").and_then(Value::as_object_mut) else {
            return root;
        };

        for versions in packages.values_mut() {
            let Some(diffs) = versions.as_array_mut() else {
                continue;
            };

            let mut expanded = Vec::with_capacity(diffs.len());
            let mut record = serde_json::Map::new();
            for diff in diffs.iter() {
                let Some(fields) = diff.as_object() else {
                    expanded.push(diff.clone());
                    continue;
                };
                for (key, value) in fields {
                    if value.as_str() == Some("__unset") {
                        record.remove(key);
                    } else {
                        record.insert(key.clone(), value.clone());
                    }
                }
                expanded.push(Value::Object(record.clone()));
            }
            *diffs = expanded;
        }

        repo.remove("minified");
        root
    }
}

#[derive(Error, Debug)]
pub enum ComposerError {
    #[error("Repository manifest is missing metadata url.")]
    MissingMetadataUrl,

    #[error("Pacakge is missing download url.")]
    MissingDownloadUrl,

    #[error("Pacakge not found.")]
    PackageNotFound,

    #[error(transparent)]
    UrlParseError(#[from] url::ParseError),

    #[error(transparent)]
    RequestError(#[from] reqwest::Error),

    #[error(transparent)]
    JsonError(#[from] serde_json::Error),

    #[error(transparent)]
    TreeError(#[from] crate::tree::TreeError),

    #[error(transparent)]
    ParsingError(#[from] crate::model::ParsingError),
}

#[derive(Debug)]
pub struct Composer {
    repo: Url,
    client: Client,
}

impl Composer {
    pub fn new(repo: &Url) -> Result<Self, ComposerError> {
        Ok(Self {
            repo: repo.clone(),
            client: Client::builder().user_agent("Composer").build()?,
        })
    }

    pub async fn fetch_package(
        &self,
        package: &ComposerPackageIdentifier,
    ) -> Result<(ComposerPackageMetadata, Tree), ComposerError> {
        let root_manifest: wire::ComposerPackageRepository = serde_json::from_value(wire::expand(
            self.client
                .get(self.repo.join("packages.json")?)
                .send()
                .await?
                .json()
                .await?,
        ))?;

        let Some(p) = (if let Some(p) = root_manifest.package(package) {
            Some(p).cloned()
        } else {
            let package_metadata_url = Url::parse(
                &root_manifest
                    .metadata_url
                    .ok_or(ComposerError::MissingMetadataUrl)?
                    .replace("%package%", &package.name().to_string()),
            )?;
            let package_manifest: wire::ComposerPackageRepository =
                serde_json::from_value(wire::expand(
                    self.client
                        .get(package_metadata_url)
                        .send()
                        .await?
                        .json()
                        .await?,
                ))?;

            package_manifest.package(package).cloned()
        }) else {
            return Err(ComposerError::PackageNotFound);
        };

        Ok((
            ComposerPackageMetadata {
                name: package.name().to_owned(),
                version: package.version().to_owned(),
                dependencies: itertools::chain!(
                    p.require
                        .iter()
                        .map(|(k, v)| ComposerDependency::Require((k.into(), v.into()))),
                    p.replace
                        .iter()
                        .map(|(k, v)| ComposerDependency::Replace((k.into(), v.into()))),
                    p.conflict
                        .iter()
                        .map(|(k, v)| ComposerDependency::Conflict((k.into(), v.into()))),
                    p.provide
                        .iter()
                        .map(|(k, v)| ComposerDependency::Provide((k.into(), v.into()))),
                )
                .collect(),
                autoload: itertools::chain!(
                    p.autoload
                        .as_ref()
                        .into_iter()
                        .flat_map(|a| a.psr_4.iter())
                        .flat_map(|(k, v)| {
                            match v {
                                wire::StringOrArray::Array(vs) => vs.as_slice(),
                                wire::StringOrArray::String(s) => std::slice::from_ref(s),
                            }
                            .iter()
                            .map(move |s| ComposerAutoloadEntry::Psr4((k.into(), s.into())))
                        }),
                    p.autoload
                        .as_ref()
                        .into_iter()
                        .flat_map(|a| a.psr_0.iter())
                        .flat_map(|(k, v)| {
                            match v {
                                wire::StringOrArray::Array(vs) => vs.as_slice(),
                                wire::StringOrArray::String(s) => std::slice::from_ref(s),
                            }
                            .iter()
                            .map(move |s| ComposerAutoloadEntry::Psr0((k.into(), s.into())))
                        }),
                    p.autoload
                        .as_ref()
                        .into_iter()
                        .flat_map(|a| a.classmap.iter())
                        .map(move |s| ComposerAutoloadEntry::Class(s.into())),
                    p.autoload
                        .as_ref()
                        .into_iter()
                        .flat_map(|a| a.files.iter())
                        .map(move |s| ComposerAutoloadEntry::File(s.into())),
                )
                .collect(),
            },
            if let Some(url) = p.download_url() {
                Tree::from_zip(&self.client.get(url).send().await?.bytes().await?)?
            } else {
                let mut tree = Tree::new();
                tree.insert(
                    "composer.json".into(),
                    Node::File(serde_json::to_string(&p)?.into()),
                )?;
                tree
            },
        ))
    }
}

pub fn build_repository(
    index: &[(ComposerPackageMetadata, StorePath)],
) -> Result<Tree, ComposerError> {
    let root_manifest = wire::ComposerPackageRepository {
        packages: Some(wire::Packages::Object(
            index
                .iter()
                .sorted_by_key(|(ComposerPackageMetadata { name, .. }, _)| name)
                .chunk_by(|(ComposerPackageMetadata { name, .. }, _)| name)
                .into_iter()
                .map(|(name, versions)| {
                    (
                        name.to_string(),
                                          wire::PackageVersions::Object(
                        versions
                            .map(|(meta, path)| {
                                (
                                    meta.version.to_string(),
                                    wire::Package {
                                        name: Some(meta.name.to_string()),
                                        version: Some(meta.version.to_string()),
                                        require: meta
                                            .dependencies
                                            .iter()
                                            .filter_map(|dep| match dep {
                                                crate::model::ComposerDependency::Require((k, v)) => {
                                                    Some((k.clone(), v.clone()))
                                                }
                                                _ => None,
                                            })
                                            .collect(),
                                        replace: meta
                                            .dependencies
                                            .iter()
                                            .filter_map(|dep| match dep {
                                                crate::model::ComposerDependency::Replace((k, v)) => {
                                                    Some((k.clone(), v.clone()))
                                                }
                                                _ => None,
                                            })
                                            .collect(),
                                        conflict: meta
                                            .dependencies
                                            .iter()
                                            .filter_map(|dep| match dep {
                                                crate::model::ComposerDependency::Conflict((k, v)) => {
                                                    Some((k.clone(), v.clone()))
                                                }
                                                _ => None,
                                            })
                                            .collect(),
                                        provide: meta
                                            .dependencies
                                            .iter()
                                            .filter_map(|dep| match dep {
                                                crate::model::ComposerDependency::Provide((k, v)) => {
                                                    Some((k.clone(), v.clone()))
                                                }
                                                _ => None,
                                            })
                                            .collect(),
                                        autoload: Some(wire::Autoload {
                                            psr_4: meta
                                                .autoload
                                                .iter()
                                                .fold(BTreeMap::<String, Vec<String>>::new(), |mut acc, entry| {
                                                    if let crate::model::ComposerAutoloadEntry::Psr4((ns, dir)) = entry {
                                                        acc.entry(ns.clone()).or_default().push(dir.clone());
                                                    }
                                                    acc
                                                })
                                                .into_iter()
                                                .map(|(k, v)| (k, wire::StringOrArray::Array(v)))
                                                .collect(),
                                            psr_0: meta
                                                .autoload
                                                .iter()
                                                .fold(BTreeMap::<String, Vec<String>>::new(), |mut acc, entry| {
                                                    if let crate::model::ComposerAutoloadEntry::Psr0((ns, dir)) = entry {
                                                        acc.entry(ns.clone()).or_default().push(dir.clone());
                                                    }
                                                    acc
                                                })
                                                .into_iter()
                                                .map(|(k, v)| (k, wire::StringOrArray::Array(v)))
                                                .collect(),
                                            classmap: meta
                                                .autoload
                                                .iter()
                                                .filter_map(|entry| match entry {
                                                    crate::model::ComposerAutoloadEntry::Class(c) => Some(c.clone()),
                                                    _ => None,
                                                })
                                                .collect(),
                                            files: meta
                                                .autoload
                                                .iter()
                                                .filter_map(|entry| match entry {
                                                    crate::model::ComposerAutoloadEntry::File(f) => Some(f.clone()),
                                                    _ => None,
                                                })
                                                .collect(),
                                            exclude_from_classmap: vec![],
                                        }),
                                        dist: Some(wire::Dist {
                                            type_: Some("path".to_string()),
                                            url: Some(path.to_absolute_path().clone()),
                                            ..Default::default()
                                        }),
                                        ..Default::default()
                                    },
                                )
                            })
                            .collect(),
                    ),
                    )
                })
                .collect(),
        )),
        ..Default::default()
    };

    Ok(Tree::new_file(&serde_json::to_vec(&root_manifest)?))
}
