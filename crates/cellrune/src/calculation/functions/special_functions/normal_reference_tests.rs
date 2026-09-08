use super::standard_normal_inverse;
use crate::calculation::limits::CalculationLimitKind;
use crate::calculation::value::ErrorKind;

// Independent mpmath 1.3.0, mp.dps=400, sqrt(2)*erfinv(2*mp.mpf(p)-1).
// mp.mpf receives the exact binary f64 input, including the subnormal rows.
const REFERENCES: [(f64, f64); 18] = [
    (f64::from_bits(1), -38.467405617144346),
    (f64::MIN_POSITIVE, -37.5193793471445),
    (1e-300, -37.0470962993612),
    (1e-100, -21.273453560965324),
    (1e-20, -9.262340089798408),
    (1e-12, -7.034483825301132),
    (0.001, -3.0902323061678135),
    (0.1, -1.2815515655446004),
    (0.25, -0.6744897501960817),
    (0.24999999999999997, -0.6744897501960818),
    (0.25000000000000006, -0.6744897501960816),
    (6.22096057427178e-16, -8.0),
    (6.22096057427179e-16, -8.0),
    (6.22096057427177e-16, -8.0),
    (0.49999999999999994, -1.3914582123358835e-16),
    (0.5000000000000001, 2.782916424671767e-16),
    (0.975, 1.9599639845400538),
    (0.9999999999999999, 8.209536151601387),
];

#[test]
fn inverse_matches_independent_reference_from_center_to_subnormal_tail() {
    for (probability, expected) in REFERENCES {
        let mut work = 0;
        let actual = standard_normal_inverse(probability, || {
            work += 1;
            Ok(())
        })
        .unwrap();
        assert!(
            (actual - expected).abs() <= 2e-15 * expected.abs(),
            "p={probability}: {actual} != {expected}"
        );
        assert!(work <= 80 * 33);
    }
    assert_eq!(standard_normal_inverse(0.5, || Ok(())), Ok(0.0));
}

#[test]
fn inverse_is_monotone_across_binary_exponents_and_central_probabilities() {
    let mut previous = f64::NEG_INFINITY;
    for exponent in -1074..=-1 {
        let probability = if exponent < -1022 {
            f64::from_bits(1_u64 << (exponent + 1074))
        } else {
            2.0_f64.powi(exponent)
        };
        let actual = standard_normal_inverse(probability, || Ok(())).unwrap();
        assert!(actual.is_finite() && actual > previous, "p={probability}");
        previous = actual;
    }
    previous = f64::NEG_INFINITY;
    for index in 1..4096 {
        let probability = f64::from(index) / 4096.0;
        let actual = standard_normal_inverse(probability, || Ok(())).unwrap();
        let reflected = standard_normal_inverse(1.0 - probability, || Ok(())).unwrap();
        assert!(actual > previous, "p={probability}");
        assert_eq!(actual, -reflected);
        previous = actual;
    }
}

#[test]
fn inverse_rejects_invalid_probabilities() {
    for probability in [
        0.0,
        1.0,
        -1.0,
        2.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert_eq!(
            standard_normal_inverse(probability, || Ok(())),
            Err(ErrorKind::Num)
        );
    }
}

#[test]
fn inverse_propagates_callback_failure_before_and_during_work() {
    let failure = ErrorKind::ResourceLimit(CalculationLimitKind::FunctionIterations);
    for probability in [0.5, 0.1, f64::from_bits(1)] {
        assert_eq!(
            standard_normal_inverse(probability, || Err(failure)),
            Err(failure)
        );
    }
    for limit in [1, 2, 16, 33] {
        let mut calls = 0;
        let actual = standard_normal_inverse(f64::from_bits(1), || {
            calls += 1;
            if calls > limit { Err(failure) } else { Ok(()) }
        });
        assert_eq!(actual, Err(failure));
        assert_eq!(calls, limit + 1);
    }
}

#[test]
fn inverse_preserves_adjacent_probabilities_near_the_median() {
    let center = 0.5_f64.to_bits();
    let mut previous = f64::NEG_INFINITY;
    for bits in (center - 1024)..=(center + 1024) {
        let probability = f64::from_bits(bits);
        let actual = standard_normal_inverse(probability, || Ok(())).unwrap();
        assert!(
            actual > previous,
            "p={probability}: {previous} then {actual}"
        );
        previous = actual;
    }
}
