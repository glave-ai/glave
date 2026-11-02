//! Whether a form seems to ask for a person's way in to something: a password, key, token, secret
//! or credential, which a form is never the place for, whoever asks. A service that hides what it
//! asks for gets past this; a service that says so does not.

use glave_event::{ServiceField, ServiceFieldKind};

use glave_event::seen as shown;

/// Whether `field` takes what a person types: a choice or a yes or no gives nothing away.
pub(super) fn can_be_typed(field: &ServiceField) -> bool {
    matches!(
        field.kind,
        ServiceFieldKind::Text { .. } | ServiceFieldKind::Number { .. }
    )
}

/// Whether `field`'s name, title or description seems to ask for a secret.
pub(super) fn field_asks_for_secret(field: &ServiceField) -> bool {
    let says = |asks: fn(&str) -> bool| {
        asks(&field.name) || asks(&field.label) || field.description.as_deref().is_some_and(asks)
    };
    words(&field.name) == ["key"]
        || says(asks_for_secret)
        || (can_be_typed(field) && says(typed_asks_for_secret))
}

/// Words that ask for a secret wherever they stand alone.
const SECRET_WORDS: [&str; 14] = [
    "pwd",
    "secret",
    "secrets",
    "token",
    "tokens",
    "credential",
    "credentials",
    "otp",
    "totp",
    "mfa",
    "cvv",
    "cvc",
    "ssn",
    "bearer",
];

/// What asks for a secret anywhere in a text's letters, however it is spaced, cased or hidden:
/// `accesstoken`, `client_secret`, `passWord1`. Long enough that no plain word holds one, as
/// `tokenizer` holds `token`. Some languages' words beside English's.
const SECRET_LETTERS: [&str; 35] = [
    "password",
    "2fa",
    "passwd",
    "passphrase",
    "passcode",
    "passwort",
    "kennwort",
    "contrasena",
    "contraseña",
    "motdepasse",
    "senha",
    "wachtwoord",
    "accesstoken",
    "authtoken",
    "idtoken",
    "refreshtoken",
    "bearertoken",
    "sessiontoken",
    "apitoken",
    "clientsecret",
    "apisecret",
    "secretkey",
    "apikey",
    "accesskey",
    "authkey",
    "privatekey",
    "signingkey",
    "encryptionkey",
    "sshkey",
    "deploykey",
    "gpgkey",
    "pgpkey",
    "clientkey",
    "licensekey",
    "verificationcode",
];

/// Words that ask for a secret only where it could be typed: "Did the tests pass?" and "Pin this
/// issue?" are a yes or no, and give nothing away.
const TYPED_SECRET_WORDS: [&str; 2] = ["pass", "pin"];

/// What, in a text's letters, asks for a secret only where it could be typed.
const TYPED_SECRET_LETTERS: [&str; 4] = ["securitycode", "onetimecode", "logincode", "authcode"];

/// What a key may be named for and be no secret, where it could be typed: an issue's or a sort's.
const PLAIN_KEYS: [&str; 3] = ["issue", "sort", "primary"];

/// Words that end in `key`, and have nothing to do with one.
const WORDS_NOT_KEYS: [&str; 6] = ["monkey", "donkey", "turkey", "hockey", "jockey", "whiskey"];

/// Whether `text` ends in a key, read as what a person could type: `openai_key`, "Stripe key".
fn ends_in_a_key(text: &str) -> bool {
    let words = words(text);
    let Some((last, before)) = words.split_last() else {
        return false;
    };
    let name = last.strip_suffix('s').unwrap_or(last);
    let plain = |kind: &str| PLAIN_KEYS.iter().any(|plain| kind.starts_with(plain));
    let named_for = name.strip_suffix("key").filter(|kind| !kind.is_empty());
    name.ends_with("key")
        && !WORDS_NOT_KEYS.contains(&name)
        && !named_for.is_some_and(plain)
        && !before.last().is_some_and(|word| plain(word))
}

/// Whether `text` seems to ask for a secret, whatever could be answered with it.
fn asks_for_secret(text: &str) -> bool {
    let letters = collapsed(text);
    words(text).iter().any(|word| {
        SECRET_WORDS.contains(&word.as_str()) || word.ends_with("token") || word.ends_with("tokens")
    }) || SECRET_LETTERS.iter().any(|secret| letters.contains(secret))
}

/// Whether `text` seems to ask for a secret, read as what a person could type.
pub(super) fn typed_asks_for_secret(text: &str) -> bool {
    let letters = collapsed(text);
    asks_for_secret(text)
        || ends_in_a_key(text)
        || words(text)
            .iter()
            .any(|word| TYPED_SECRET_WORDS.contains(&word.as_str()))
        || TYPED_SECRET_LETTERS
            .iter()
            .any(|secret| letters.contains(secret))
}

/// `text`'s letters and digits alone, in lowercase, with what hides them gone.
pub(super) fn collapsed(text: &str) -> String {
    text.chars()
        .filter(|letter| letter.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `text`'s words in lowercase, split at anything not a letter or a digit, where a lowercase
/// letter meets a capital (`apiKey`, `client_secret`), and where digits start or end (`PIN1` is
/// `pin`, and `1`).
fn words(text: &str) -> Vec<String> {
    let mut words = vec![String::new()];
    let mut lower = false;
    let mut digit = false;
    let mut alphabetic = false;
    for letter in text.chars().filter(|letter| shown(*letter)) {
        if !letter.is_alphanumeric() {
            words.push(String::new());
            lower = false;
            digit = false;
            alphabetic = false;
            continue;
        }
        if (lower && letter.is_uppercase())
            || (alphabetic && letter.is_ascii_digit())
            || (digit && !letter.is_ascii_digit())
        {
            words.push(String::new());
        }
        lower = letter.is_lowercase();
        digit = letter.is_ascii_digit();
        alphabetic = letter.is_alphabetic();
        if let Some(word) = words.last_mut() {
            word.extend(letter.to_lowercase());
        }
    }
    words.retain(|word| !word.is_empty());
    words
}
