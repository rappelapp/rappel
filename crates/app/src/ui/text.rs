//! Texts with numbers: grouped digits and counted nouns. Pure, so they
//! are unit-tested. English only until translations exist (SPEC §5).

/// `n` with its digits in groups of three: 5210 → "5,210".
pub fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `n` and the noun, singular for exactly one: "1 deck", "1,200 cards".
pub fn count(n: usize, singular: &str, plural: &str) -> String {
    let noun = if n == 1 { singular } else { plural };
    format!("{} {noun}", group_digits(n))
}

/// The non-zero counts of decks, cards and reviews as one phrase:
/// "12 decks, 840 cards and 5,210 reviews", "1 card", or `None` if all
/// are zero.
pub fn decks_cards_reviews(decks: usize, cards: usize, reviews: usize) -> Option<String> {
    let parts: Vec<String> = [
        (decks, "deck", "decks"),
        (cards, "card", "cards"),
        (reviews, "review", "reviews"),
    ]
    .into_iter()
    .filter(|&(n, _, _)| n > 0)
    .map(|(n, singular, plural)| count(n, singular, plural))
    .collect();
    match parts.as_slice() {
        [] => None,
        [one] => Some(one.clone()),
        [init @ .., last] => Some(format!("{} and {last}", init.join(", "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_grouped_by_three() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1_000), "1,000");
        assert_eq!(group_digits(5_210), "5,210");
        assert_eq!(group_digits(1_048_576), "1,048,576");
    }

    #[test]
    fn one_is_singular() {
        assert_eq!(count(1, "deck", "decks"), "1 deck");
        assert_eq!(count(0, "deck", "decks"), "0 decks");
        assert_eq!(count(1_200, "card", "cards"), "1,200 cards");
    }

    #[test]
    fn zero_counts_are_left_out() {
        assert_eq!(
            decks_cards_reviews(12, 840, 5_210).as_deref(),
            Some("12 decks, 840 cards and 5,210 reviews")
        );
        assert_eq!(
            decks_cards_reviews(1, 0, 1).as_deref(),
            Some("1 deck and 1 review")
        );
        assert_eq!(decks_cards_reviews(0, 1, 0).as_deref(), Some("1 card"));
        assert_eq!(decks_cards_reviews(0, 0, 0), None);
    }
}
