#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "tests keep assertions readable alongside fallible setup"
    )]

    use core::str::FromStr as _;
    use core::time::Duration;

    use const_oid::db::rfc5280::{ID_KP_CLIENT_AUTH, ID_KP_SERVER_AUTH};
    use const_oid::db::rfc5912::ECDSA_WITH_SHA_256;
    use der::{DecodePem as _, EncodePem as _, asn1::BitString, pem::LineEnding};
    use pki::{
        cert, csr,
        error::{PkiError, Result},
        key::{Signature, Signer},
        pem,
        profile::{MuakCa, MuakClient, MuakServer},
        serial,
    };
    use signature::{Keypair as _, Signer as _};
    use spki::{
        DynSignatureAlgorithmIdentifier as _, EncodePublicKey as _, SignatureBitStringEncoding as _,
    };
    use x509_cert::{
        Certificate,
        builder::{Builder as _, CertificateBuilder, profile::BuilderProfile},
        ext::pkix::{BasicConstraints, ExtendedKeyUsage, SubjectAltName},
        name::Name,
        request::CertReq,
        time::Validity,
    };

    fn make_test_ca() -> Result<(String, String, Signer, Certificate)> {
        let (signer, cert) = cert::generate_ca("Test CA")?;
        let cert_pem = cert.to_pem(LineEnding::LF)?;
        let key_pem = pem::encode_pkcs8(signer.pkcs8_der())?;
        Ok((key_pem, cert_pem, signer, cert))
    }

    fn make_csr_test_ca() -> Result<(String, Certificate)> {
        let (signer, certificate) = cert::generate_ca("CSR Test CA")?;
        Ok((pem::encode_pkcs8(signer.pkcs8_der())?, certificate))
    }

    #[test]
    fn public_api_generate_ca_and_server_cert() -> Result<()> {
        // ARRANGE
        let (ca_key_pem, ca_cert_pem, ca_signer, ca_cert) = make_test_ca()?;

        // ACT
        let (server_key, server_cert) = cert::generate_server("muak-server", &ca_signer, &ca_cert)?;
        let server_cert_pem = server_cert.to_pem(LineEnding::LF)?;
        let server_key_pem = pem::encode_pkcs8(server_key.pkcs8_der())?;
        let fingerprint = cert::compute_fingerprint(&server_cert)?;

        // ASSERT
        assert!(ca_cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca_key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(server_cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(server_key_pem.contains("BEGIN PRIVATE KEY"));
        assert_eq!(fingerprint.len(), 64);

        Ok(())
    }

    #[test]
    fn public_api_generate_and_sign_csr() -> Result<()> {
        // ARRANGE
        let (ca_key_pem, _, _, ca_cert) = make_test_ca()?;
        let (key_pem, csr_pem) = csr::generate("test-client")?;
        let csr_fp = csr::compute_fingerprint(&csr_pem)?;

        // ACT
        let (cert, cert_fp) = csr::sign(&csr_pem, &ca_key_pem, &ca_cert)?;
        let cert_pem = cert.to_pem(LineEnding::LF)?;

        // ASSERT
        assert!(!key_pem.is_empty());
        assert!(!csr_pem.is_empty());
        assert_eq!(csr_fp.len(), 64);
        assert_eq!(cert_fp.len(), 64);
        assert!(cert_pem.contains("BEGIN CERTIFICATE"));

        Ok(())
    }

    #[test]
    fn public_api_load_ca_from_pem() -> Result<()> {
        // ARRANGE
        let (ca_key_pem, ca_cert_pem, _, _) = make_test_ca()?;
        let loaded_cert = Certificate::from_pem(&ca_cert_pem)?;
        let (_, csr_pem) = csr::generate("test-client")?;

        // ACT
        let (_, fp) = csr::sign(&csr_pem, &ca_key_pem, &loaded_cert)?;

        // ASSERT
        assert_eq!(fp.len(), 64);

        Ok(())
    }

    #[test]
    fn cert_generate_ca_certificate_is_self_signed_ca() -> Result<()> {
        // ARRANGE
        let common_name = "Test CA";

        // ACT
        let (_signer, certificate) = cert::generate_ca(common_name)?;

        // ASSERT
        assert_eq!(
            certificate.tbs_certificate().issuer(),
            certificate.tbs_certificate().subject()
        );
        let (_, constraints) = certificate
            .tbs_certificate()
            .get_extension::<BasicConstraints>()?
            .ok_or(PkiError::CsrVerification)?;
        assert!(constraints.ca);

        Ok(())
    }

    #[test]
    fn cert_generate_server_certificate_sets_server_extensions() -> Result<()> {
        // ARRANGE
        let (ca_signer, ca_certificate) = cert::generate_ca("Test CA")?;

        // ACT
        let (_server_signer, server_certificate) =
            cert::generate_server("muak-server", &ca_signer, &ca_certificate)?;

        // ASSERT
        let (_, constraints) = server_certificate
            .tbs_certificate()
            .get_extension::<BasicConstraints>()?
            .ok_or(PkiError::CsrVerification)?;
        let (_, san) = server_certificate
            .tbs_certificate()
            .get_extension::<SubjectAltName>()?
            .ok_or(PkiError::CsrVerification)?;
        let (_, eku) = server_certificate
            .tbs_certificate()
            .get_extension::<ExtendedKeyUsage>()?
            .ok_or(PkiError::CsrVerification)?;
        assert!(!constraints.ca);
        assert_eq!(san.0.len(), 2);
        assert!(eku.0.contains(&ID_KP_SERVER_AUTH));

        Ok(())
    }

    #[test]
    fn cert_compute_cert_fingerprint_is_stable() -> Result<()> {
        // ARRANGE
        let (_signer, certificate) = cert::generate_ca("Fingerprint CA")?;

        // ACT
        let fingerprint_a = cert::compute_fingerprint(&certificate)?;
        let fingerprint_b = cert::compute_fingerprint(&certificate)?;

        // ASSERT
        assert_eq!(fingerprint_a, fingerprint_b);
        assert_eq!(fingerprint_a.len(), 64);

        Ok(())
    }

    #[test]
    fn cert_generate_ca_certificate_rejects_invalid_common_name() {
        // ARRANGE
        let common_name = "foo+";

        // ACT
        let result = cert::generate_ca(common_name);

        // ASSERT
        let _error = result.map(|_| ()).unwrap_err();
    }

    #[test]
    fn cert_generate_server_certificate_rejects_invalid_common_name() -> Result<()> {
        // ARRANGE
        let (ca_signer, ca_certificate) = cert::generate_ca("Test CA")?;

        // ACT
        let result = cert::generate_server("foo+", &ca_signer, &ca_certificate);

        // ASSERT
        let _error = result.map(|_| ()).unwrap_err();

        Ok(())
    }

    #[test]
    fn csr_generate_and_sign_produce_client_certificate() -> Result<()> {
        // ARRANGE
        let (ca_key_pem, ca_certificate) = make_csr_test_ca()?;
        let (_client_key_pem, csr_pem) = csr::generate("client-1")?;

        // ACT
        let (certificate, fingerprint) = csr::sign(&csr_pem, &ca_key_pem, &ca_certificate)?;

        // ASSERT
        let (_, eku) = certificate
            .tbs_certificate()
            .get_extension::<ExtendedKeyUsage>()?
            .ok_or(PkiError::CsrVerification)?;
        assert_eq!(fingerprint.len(), 64);
        assert_eq!(eku.0.len(), 1);

        Ok(())
    }

    #[test]
    fn csr_sign_rejects_tampered_signature() -> Result<()> {
        // ARRANGE
        let (ca_key_pem, ca_certificate) = make_csr_test_ca()?;
        let (_client_key_pem, csr_pem) = csr::generate("client-2")?;
        let mut request = CertReq::from_pem(&csr_pem)?;
        let mut signature_bytes = request
            .signature
            .as_bytes()
            .ok_or(PkiError::CsrVerification)?
            .to_vec();
        let last_byte = signature_bytes
            .last_mut()
            .ok_or(PkiError::CsrVerification)?;
        *last_byte ^= 0x01;
        request.signature = BitString::from_bytes(&signature_bytes)?;
        let tampered_csr_pem = request.to_pem(LineEnding::LF)?;

        // ACT
        let result = csr::sign(&tampered_csr_pem, &ca_key_pem, &ca_certificate);

        // ASSERT
        assert!(matches!(result, Err(PkiError::CsrVerification)));

        Ok(())
    }

    #[test]
    fn csr_sign_rejects_invalid_ca_key_pem() -> Result<()> {
        // ARRANGE
        let (_ca_key_pem, ca_certificate) = make_csr_test_ca()?;
        let (_client_key_pem, csr_pem) = csr::generate("client-3")?;

        // ACT
        let result = csr::sign(
            &csr_pem,
            "-----BEGIN PRIVATE KEY-----\ninvalid\n-----END PRIVATE KEY-----\n",
            &ca_certificate,
        );

        // ASSERT
        assert!(matches!(
            result,
            Err(PkiError::Der(_) | PkiError::InvalidKeyEncoding)
        ));

        Ok(())
    }

    #[test]
    fn csr_compute_fingerprint_rejects_invalid_pem() {
        // ARRANGE
        let invalid_csr_pem =
            "-----BEGIN CERTIFICATE REQUEST-----\ninvalid\n-----END CERTIFICATE REQUEST-----\n";

        // ACT
        let result = csr::compute_fingerprint(invalid_csr_pem);

        // ASSERT
        let _error = result.unwrap_err();
    }

    #[test]
    fn csr_generate_rejects_invalid_common_name() {
        // ARRANGE
        let common_name = "foo+";

        // ACT
        let result = csr::generate(common_name);

        // ASSERT
        let _error = result.unwrap_err();
    }

    #[test]
    fn key_generate_and_reload_preserves_public_key() -> Result<()> {
        // ARRANGE
        let signer = Signer::generate()?;

        // ACT
        let reloaded_signer = Signer::from_pkcs8_der(signer.pkcs8_der())?;

        // ASSERT
        assert_eq!(
            signer.public_key_bytes(),
            reloaded_signer.public_key_bytes()
        );

        Ok(())
    }

    #[test]
    fn key_verifying_key_and_signature_algorithm_encode_as_expected() -> Result<()> {
        // ARRANGE
        let signer = Signer::generate()?;

        // ACT
        let public_key_document = signer.verifying_key().to_public_key_der()?;
        let algorithm_identifier = signer.signature_algorithm_identifier()?;

        // ASSERT
        assert!(!public_key_document.as_bytes().is_empty());
        assert_eq!(algorithm_identifier.oid, ECDSA_WITH_SHA_256);

        Ok(())
    }

    #[test]
    fn key_signer_produces_non_empty_signature() -> Result<()> {
        // ARRANGE
        let signer = Signer::generate()?;

        // ACT
        let signature = signer
            .try_sign(b"message")
            .map_err(|_sign_error| PkiError::KeyGeneration)?;

        // ASSERT
        assert!(!signature.0.is_empty());
        assert!(!signature.to_bitstring()?.raw_bytes().is_empty());

        Ok(())
    }

    #[test]
    fn key_from_pkcs8_der_rejects_invalid_bytes() {
        // ARRANGE
        let invalid_pkcs8_der = [0_u8; 8];

        // ACT
        let result = Signer::from_pkcs8_der(&invalid_pkcs8_der);

        // ASSERT
        assert!(matches!(result, Err(PkiError::InvalidKeyEncoding)));
    }

    #[test]
    fn key_signature_bitstring_contains_original_bytes() -> Result<()> {
        // ARRANGE
        let signature = Signature(vec![1, 2, 3, 4]);

        // ACT
        let bitstring = signature.to_bitstring()?;

        // ASSERT
        assert_eq!(bitstring.raw_bytes(), &[1, 2, 3, 4]);

        Ok(())
    }

    fn build_server_certificate(dns_names: Vec<String>) -> Result<Certificate> {
        let (ca_signer, ca_certificate) = cert::generate_ca("Profile Test CA")?;
        let server_signer = Signer::generate()?;
        let profile = MuakServer {
            issuer: ca_certificate.tbs_certificate().subject().clone(),
            subject: Name::from_str("CN=server-profile,O=Muak")?,
            dns_names,
        };
        let serial = serial::generate()?;
        let validity = Validity::from_now(Duration::from_secs(cert::CERT_VALIDITY_SECS))?;
        let spki = serial::signer_spki(&server_signer)?;
        Ok(CertificateBuilder::new(profile, serial, validity, spki)?
            .build::<_, Signature>(&ca_signer)?)
    }

    #[test]
    fn profile_server_profile_without_dns_names_omits_san_extension() -> Result<()> {
        // ARRANGE
        let dns_names = Vec::new();

        // ACT
        let certificate = build_server_certificate(dns_names)?;

        // ASSERT
        assert!(
            certificate
                .tbs_certificate()
                .get_extension::<SubjectAltName>()?
                .is_none()
        );

        Ok(())
    }

    #[test]
    fn profile_server_profile_with_only_invalid_dns_names_omits_san_extension() -> Result<()> {
        // ARRANGE
        let dns_names = vec!["bad-\u{0100}".to_owned()];

        // ACT
        let certificate = build_server_certificate(dns_names)?;

        // ASSERT
        assert!(
            certificate
                .tbs_certificate()
                .get_extension::<SubjectAltName>()?
                .is_none()
        );

        Ok(())
    }

    #[test]
    fn profile_ca_and_client_profile_subject_and_issuer_accessors_return_expected_names()
    -> Result<()> {
        // ARRANGE
        let subject = Name::from_str("CN=profile-subject,O=Muak")?;
        let issuer = Name::from_str("CN=profile-issuer,O=Muak")?;
        let ca_profile = MuakCa {
            subject: subject.clone(),
        };
        let client_profile = MuakClient {
            issuer: issuer.clone(),
            subject: subject.clone(),
        };

        // ACT
        let ca_issuer = BuilderProfile::get_issuer(&ca_profile, &subject);
        let ca_subject = BuilderProfile::get_subject(&ca_profile);
        let client_issuer = BuilderProfile::get_issuer(&client_profile, &subject);
        let client_subject = BuilderProfile::get_subject(&client_profile);

        // ASSERT
        assert_eq!(ca_issuer, subject);
        assert_eq!(ca_subject, subject);
        assert_eq!(client_issuer, issuer);
        assert_eq!(client_subject, subject);

        Ok(())
    }

    #[test]
    fn profile_client_profile_builds_client_auth_extended_key_usage() -> Result<()> {
        // ARRANGE
        let (ca_signer, ca_certificate) = cert::generate_ca("Profile Test CA")?;
        let client_signer = Signer::generate()?;
        let profile = MuakClient {
            issuer: ca_certificate.tbs_certificate().subject().clone(),
            subject: Name::from_str("CN=client-profile,O=Muak")?,
        };
        let serial = serial::generate()?;
        let validity = Validity::from_now(Duration::from_secs(cert::CERT_VALIDITY_SECS))?;
        let spki = serial::signer_spki(&client_signer)?;

        // ACT
        let certificate = CertificateBuilder::new(profile, serial, validity, spki)?
            .build::<_, Signature>(&ca_signer)?;

        // ASSERT
        let eku = certificate
            .tbs_certificate()
            .get_extension::<ExtendedKeyUsage>()?
            .ok_or(PkiError::CsrVerification)?;
        assert_eq!(eku.1.0, vec![ID_KP_CLIENT_AUTH]);

        Ok(())
    }

    #[test]
    fn profile_server_profile_builds_server_auth_extended_key_usage() -> Result<()> {
        // ARRANGE
        let dns_names = vec!["server.example".to_owned()];

        // ACT
        let certificate = build_server_certificate(dns_names)?;

        // ASSERT
        let eku = certificate
            .tbs_certificate()
            .get_extension::<ExtendedKeyUsage>()?
            .ok_or(PkiError::CsrVerification)?;
        assert_eq!(eku.1.0, vec![ID_KP_SERVER_AUTH]);

        Ok(())
    }

    #[test]
    fn util_pem_roundtrip_and_signer_loading_work() -> Result<()> {
        // ARRANGE
        let signer = Signer::generate()?;

        // ACT
        let pem_doc = pem::encode_pkcs8(signer.pkcs8_der())?;
        let der = pem::decode_pkcs8(&pem_doc)?;
        let loaded_signer = pem::load_signer(&pem_doc)?;

        // ASSERT
        assert_eq!(der, signer.pkcs8_der());
        assert_eq!(loaded_signer.public_key_bytes(), signer.public_key_bytes());

        Ok(())
    }

    #[test]
    fn util_get_spki_and_generate_serial_return_non_empty_values() -> Result<()> {
        // ARRANGE
        let signer = Signer::generate()?;

        // ACT
        let spki = serial::signer_spki(&signer)?;
        let serial = serial::generate()?;

        // ASSERT
        assert!(!spki.subject_public_key.raw_bytes().is_empty());
        assert!(!serial.as_bytes().is_empty());

        Ok(())
    }

    #[test]
    fn util_invalid_pem_inputs_are_rejected() {
        // ARRANGE
        let invalid_pem = "-----BEGIN PRIVATE KEY-----\ninvalid\n-----END PRIVATE KEY-----\n";

        // ACT
        let der_result = pem::decode_pkcs8(invalid_pem);
        let signer_result = pem::load_signer(invalid_pem);

        // ASSERT
        let _der_error = der_result.unwrap_err();
        let _signer_error = signer_result.map(|_| ()).unwrap_err();
    }
}
