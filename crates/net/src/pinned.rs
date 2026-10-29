// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Trust for one account's server by certificate fingerprint (#252, E5): a verifier that accepts
//! what the system trusts, or else exactly the one certificate whose SHA-256 fingerprint the
//! owner chose to trust for the account.
//!
//! It is the system's own verifier ([`crate::tls`], `rustls-platform-verifier`) with one extra
//! way in, and the extra way is narrow:
//!
//! - It opens only when the system said the issuer is unknown (or, on a system that doesn't say
//!   why, that the chain is not trusted): never for a certificate that is expired, not yet valid,
//!   revoked, for another name or badly signed. The dates and the name are checked here again,
//!   whatever error the system gave, so that no system's wording can let an expired or wrong-host
//!   certificate through.
//! - It compares the fingerprint of the end-entity certificate, the one the server shows first,
//!   with the one the owner chose (all 32 bytes; a fingerprint is public, so no care over timing). The server must still prove
//!   it holds the certificate's key: the handshake signature is checked by the system's
//!   algorithms, as for any certificate.
//! - It is never a switch that turns checking off. No fingerprint set means the system alone
//!   decides, and the verifier then only notes what it saw.
//!
//! When the certificate is not accepted, the verifier notes why ([`CertificateProblem`]) for the
//! caller to read after the handshake fails: the caller shows setup what it needs (fingerprint,
//! who it was issued to and by, dates), or says plainly that the certificate can't be trusted.

use std::fmt;
use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{CertificateError, DigitallySignedStruct, Error, SignatureScheme};

use crate::cert::{self, CertificateFault, CertificateProblem};

/// The system's verifier with the one extra way in.
pub(crate) struct Pinned {
    system: Arc<dyn ServerCertVerifier>,
    /// The SHA-256 fingerprint of the one certificate trusted besides the system's.
    pin: Option<[u8; 32]>,
    /// Why the last certificate was not accepted.
    noted: Mutex<Option<CertificateProblem>>,
}

impl fmt::Debug for Pinned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pinned").field("pin", &self.pin.is_some()).finish()
    }
}

impl Pinned {
    pub(crate) fn new(system: Arc<dyn ServerCertVerifier>, pin: Option<[u8; 32]>) -> Self {
        Self { system, pin, noted: Mutex::new(None) }
    }

    /// What the verifier noted of the certificate it refused, if it did; read once.
    pub(crate) fn take_problem(&self) -> Option<CertificateProblem> {
        self.noted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take()
    }

    fn note(&self, problem: CertificateProblem) {
        *self.noted.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(problem);
    }

    /// What to do with a certificate the system refused with `error`.
    fn refused(&self, error: Error, end_entity: &CertificateDer<'_>, server_name: &ServerName<'_>, now: UnixTime) -> Result<ServerCertVerified, Error> {
        let Some(info) = cert::describe(end_entity.as_ref()) else { return Err(error) };
        // What the system itself says is wrong with the dates or the name is believed.
        let fault = match &error {
            Error::InvalidCertificate(CertificateError::Expired | CertificateError::ExpiredContext { .. }) => Some(CertificateFault::Expired),
            Error::InvalidCertificate(CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. }) => Some(CertificateFault::NotYetValid),
            Error::InvalidCertificate(CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. }) => Some(CertificateFault::WrongName),
            // The system does not trust the chain: the one thing a fingerprint may change.
            Error::InvalidCertificate(CertificateError::UnknownIssuer | CertificateError::Other(_)) => None,
            // Anything else (a bad signature, a revocation, a purpose, a protocol error) is final.
            _ => return Err(error),
        };
        // And the dates and the name are checked again, by this program, whatever the system said.
        let now = i64::try_from(now.as_secs()).unwrap_or(i64::MAX);
        let fault = fault
            .or((now < info.not_before).then_some(CertificateFault::NotYetValid))
            .or((now > info.not_after).then_some(CertificateFault::Expired))
            .or_else(|| match ParsedCertificate::try_from(end_entity) {
                Ok(parsed) => rustls::client::verify_server_name(&parsed, server_name).err().map(|_| CertificateFault::WrongName),
                Err(_) => Some(CertificateFault::WrongName),
            });
        if let Some(fault) = fault {
            self.note(CertificateProblem::Unacceptable(fault, info));
            return Err(error);
        }
        match self.pin {
            Some(pin) if pin == cert::sha256(end_entity.as_ref()) => Ok(ServerCertVerified::assertion()),
            Some(_) => {
                self.note(CertificateProblem::Changed(info));
                Err(error)
            }
            None => {
                self.note(CertificateProblem::NotTrusted(info));
                Err(error)
            }
        }
    }
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        match self.system.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now) {
            Ok(verified) => Ok(verified),
            Err(error) => self.refused(error, end_entity, server_name, now),
        }
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        self.system.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        self.system.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.system.supported_verify_schemes()
    }
}
