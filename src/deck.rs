//! The shuffle bag that makes sure a photo is never dealt twice in a round.

use std::path::{Path, PathBuf};

use rand::Rng;
use rand::seq::SliceRandom;

/// A shuffled queue of photos.
///
/// Photos are popped off the end of the queue. Once the queue is empty a round
/// is complete and the caller can build a fresh one.
#[derive(Clone, Debug)]
pub struct Round {
    pub number: usize,
    /// Number of photos that can still be dealt in this round.
    pub total: usize,
    /// Number of photos that have actually been shown so far.
    pub dealt: usize,
    remaining: Vec<PathBuf>,
}

impl Round {
    /// Build a new shuffled round.
    ///
    /// If `avoid_immediate` is the first card in the new shuffle, it is swapped
    /// with another card so that a new round doesn't open with the exact same
    /// photo that just closed the previous one.
    pub fn new(
        photos: &[PathBuf],
        number: usize,
        avoid_immediate: Option<&Path>,
        rng: &mut impl Rng,
    ) -> Self {
        let mut remaining = photos.to_vec();
        remaining.shuffle(rng);

        if remaining.len() > 1
            && let Some(avoid) = avoid_immediate
            && remaining.last().map(PathBuf::as_path) == Some(avoid)
        {
            let last = remaining.len() - 1;
            let swap_with = rng.random_range(0..last);
            remaining.swap(last, swap_with);
        }

        Self {
            number,
            total: remaining.len(),
            dealt: 0,
            remaining,
        }
    }

    /// Deal the next photo.
    pub fn next(&mut self) -> Option<PathBuf> {
        self.remaining.pop()
    }

    /// Record that the last dealt photo was shown successfully.
    pub fn mark_dealt(&mut self) {
        self.dealt = self.dealt.saturating_add(1);
    }

    /// A photo was in this round but can't be read. Remove it from the
    /// denominator so the "x / y" counter stays honest.
    pub fn mark_unreadable(&mut self) {
        self.total = self.total.saturating_sub(1);
    }

    pub fn is_complete(&self) -> bool {
        self.remaining.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::Round;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use std::path::PathBuf;

    fn photos(n: usize) -> Vec<PathBuf> {
        (0..n)
            .map(|i| PathBuf::from(format!("/photos/{i:02}.jpg")))
            .collect()
    }

    #[test]
    fn every_photo_is_dealt_exactly_once() {
        let photos = photos(20);
        let mut rng = StdRng::seed_from_u64(7);
        let mut round = Round::new(&photos, 1, None, &mut rng);

        let mut seen = Vec::new();
        while let Some(photo) = round.next() {
            assert!(!seen.contains(&photo), "dealt a duplicate: {photo:?}");
            seen.push(photo);
            round.mark_dealt();
        }

        assert_eq!(round.dealt, 20);
        assert_eq!(round.total, 20);
        assert!(round.is_complete());
        assert_eq!(seen.len(), photos.len());
        for photo in photos {
            assert!(seen.contains(&photo));
        }
    }

    #[test]
    fn new_round_does_not_start_with_the_previous_photo() {
        let photos = photos(8);
        let avoid = photos[3].clone();

        for seed in 0..100 {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut round = Round::new(&photos, 2, Some(avoid.as_path()), &mut rng);
            assert_ne!(round.next().as_deref(), Some(avoid.as_path()));
        }
    }

    #[test]
    fn a_single_photo_round_is_a_special_case() {
        let photos = photos(1);
        let mut rng = StdRng::seed_from_u64(1);
        let mut round = Round::new(&photos, 1, Some(photos[0].as_path()), &mut rng);
        assert_eq!(round.next(), Some(photos[0].clone()));
        assert!(round.is_complete());
    }

    #[test]
    fn unreadable_photos_leave_the_denominator() {
        let photos = photos(4);
        let mut rng = StdRng::seed_from_u64(3);
        let mut round = Round::new(&photos, 1, None, &mut rng);
        round.mark_unreadable();
        assert_eq!(round.total, 3);
        assert_eq!(round.dealt, 0);
    }
}
