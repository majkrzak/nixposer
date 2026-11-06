use std::{collections::HashSet, fmt::Display, str::FromStr};

use nom::Finish;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ComposerVendorName(String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ComposerProjectName(String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ComposerPackageName((ComposerVendorName, ComposerProjectName));

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ComposerPackageVersion(String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ComposerPackageIdentifier((ComposerPackageName, ComposerPackageVersion));

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ComposerDependency {
    Require((String, String)),
    Replace((String, String)),
    Conflict((String, String)),
    Provide((String, String)),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ComposerAutoloadEntry {
    Psr4((String, String)),
    Psr0((String, String)),
    Class(String),
    File(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerPackageMetadata {
    pub name: ComposerPackageName,
    pub version: ComposerPackageVersion,
    pub dependencies: HashSet<ComposerDependency>,
    pub autoload: HashSet<ComposerAutoloadEntry>,
}

mod parse {
    use nom::{
        IResult, Parser,
        branch::alt,
        bytes::complete::tag,
        character::complete::{char, digit1, one_of},
        combinator::{into, opt, recognize},
        multi::{many_m_n, many0, many1},
        sequence::{preceded, separated_pair},
    };

    use crate::model::{
        ComposerPackageIdentifier, ComposerPackageName, ComposerPackageVersion,
        ComposerProjectName, ComposerVendorName,
    };

    const AZ09: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

    /// Parse Composer Vendor Name according to the official regex:
    /// `[a-z0-9]([_.-]?[a-z0-9]+)*`
    pub(super) fn composer_vendor_name(input: &str) -> IResult<&str, ComposerVendorName> {
        into(recognize((
            one_of(AZ09),
            many0((opt(one_of("_.-")), many1(one_of(AZ09)))),
        )))
        .map(ComposerVendorName)
        .parse(input)
    }

    /// Parse Composer Project Name according to the official regex:
    /// `[a-z0-9](([_.]|-{1,2})?[a-z0-9]+)*`
    pub(super) fn composer_project_name(input: &str) -> IResult<&str, ComposerProjectName> {
        into(recognize((
            one_of(AZ09),
            many0((
                opt(alt((
                    recognize(one_of("_.")),
                    recognize(many_m_n(1, 2, char('-'))),
                ))),
                many1(one_of(AZ09)),
            )),
        )))
        .map(ComposerProjectName)
        .parse(input)
    }

    /// Parse Composer Package Name according to the official regex:
    /// `[a-z0-9]([_.-]?[a-z0-9]+)*/[a-z0-9](([_.]|-{1,2})?[a-z0-9]+)*`
    pub(super) fn composer_package_name(input: &str) -> IResult<&str, ComposerPackageName> {
        separated_pair(composer_vendor_name, char('/'), composer_project_name)
            .map(ComposerPackageName)
            .parse(input)
    }

    /// Parse Composer Package Version according to the official rules:
    /// > This must follow the format of `X.Y.Z` or `vX.Y.Z` with an optional suffix
    /// > of `-dev`, `-patch` (`-p`), `-alpha` (`-a`), `-beta` (`-b`) or `-RC`.
    /// > The patch, alpha, beta and RC suffixes can also be followed by a number.
    pub(super) fn comoser_package_version(input: &str) -> IResult<&str, ComposerPackageVersion> {
        into(recognize((
            opt(char('v')),
            separated_pair(digit1, char('.'), separated_pair(digit1, char('.'), digit1)),
            opt(preceded(
                char('-'),
                alt((
                    tag("dev"),
                    recognize((
                        alt((
                            tag("patch"),
                            tag("p"),
                            tag("alpha"),
                            tag("a"),
                            tag("beta"),
                            tag("b"),
                            tag("RC"),
                        )),
                        opt(digit1),
                    )),
                )),
            )),
        )))
        .map(ComposerPackageVersion)
        .parse(input)
    }

    /// Parse Composer Package Identifier.
    pub(super) fn composer_package_identifier(
        input: &str,
    ) -> IResult<&str, ComposerPackageIdentifier> {
        (
            composer_package_name,
            preceded(char('@'), comoser_package_version),
        )
            .map(ComposerPackageIdentifier)
            .parse(input)
    }
}

#[derive(Error, Debug)]
#[error("Parsing failed")]
pub struct ParsingError;

impl FromStr for ComposerPackageName {
    type Err = ParsingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse::composer_package_name(s).finish() {
            Ok(("", r)) => Ok(r),
            _ => Err(ParsingError),
        }
    }
}

impl FromStr for ComposerPackageVersion {
    type Err = ParsingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse::comoser_package_version(s).finish() {
            Ok(("", r)) => Ok(r),
            _ => Err(ParsingError),
        }
    }
}

impl FromStr for ComposerPackageIdentifier {
    type Err = ParsingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse::composer_package_identifier(s).finish() {
            Ok(("", r)) => Ok(r),
            _ => Err(ParsingError),
        }
    }
}

impl ComposerPackageIdentifier {
    #[must_use]
    pub const fn name(&self) -> &ComposerPackageName {
        let Self((name, _)) = self;
        name
    }
    #[must_use]
    pub const fn version(&self) -> &ComposerPackageVersion {
        let Self((_, version)) = self;
        version
    }
}

impl ComposerPackageName {
    #[must_use]
    pub const fn vendor(&self) -> &ComposerVendorName {
        let Self((vendor, _)) = self;
        vendor
    }
    #[must_use]
    pub const fn project(&self) -> &ComposerProjectName {
        let Self((_, project)) = self;
        project
    }
}

impl Display for ComposerPackageIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self((name, version)) = self;
        write!(f, "{name}/{version}")
    }
}

impl Display for ComposerPackageName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self((vendor, project)) = self;
        write!(f, "{vendor}/{project}")
    }
}

impl Display for ComposerVendorName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(vendor) = self;
        write!(f, "{vendor}")
    }
}

impl Display for ComposerProjectName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(project) = self;
        write!(f, "{project}")
    }
}

impl Display for ComposerPackageVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self(version) = self;
        write!(f, "{version}")
    }
}
