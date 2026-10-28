/// `user-card` | `user_card` | `userCard` → `UserCard`.
pub fn pascal(s: &str) -> String {
    let mut out = String::new();
    let mut up = true;
    for c in s.chars() {
        if c == '-' || c == '_' || c == ' ' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// `UserCard` | `user_card` | `userCard` → `user-card`.
pub fn kebab(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c == '_' || c == ' ' {
            out.push('-');
        } else if c.is_uppercase() {
            if i > 0 && !out.ends_with('-') {
                out.push('-');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

pub fn valid_ident(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_ascii_alphabetic()) && cs.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(pascal("user-card"), "UserCard");
        assert_eq!(pascal("home"), "Home");
        assert_eq!(kebab("UserCard"), "user-card");
        assert_eq!(kebab("user_card"), "user-card");
        assert!(valid_ident("user-card") && !valid_ident("1x") && !valid_ident("a/b"));
    }
}
