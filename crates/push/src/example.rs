//! RFC 8291's worked example (appendix A), which the tests check glave's pushes against: a
//! browser's published keys and secret, and the message an application server encrypted to them.
//! None of it is a secret: the RFC prints it for exactly this.

/// The example browser's public key, as `p256dh`.
pub(crate) const P256DH: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";

/// Its private key, which decrypts what is sent to it.
pub(crate) const PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";

/// Its authentication secret, as `auth`.
pub(crate) const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";

/// What the example says.
pub(crate) const PLAINTEXT: &[u8] = b"When I grow up, I want to be a watermelon";

/// The example's encrypted message, as the push service is sent it.
pub(crate) const MESSAGE: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";
