//! Multi-factor matching engine. Ported 1:1 from
//! `src/services/matchingService.ts` (commodity is a hard filter worth 40,
//! quantity 20, quality grade 15, availability date 15, location 10).

use crate::models::{DemandRequest, MatchFactor, SupplyListing};

pub const MIN_SCORE: i32 = 30;

pub fn compute_match(demand: &DemandRequest, listing: &SupplyListing) -> Option<(i32, Vec<MatchFactor>)> {
    let mut factors = Vec::new();
    let mut score = 0i32;

    let commodity_match = demand.commodity == listing.commodity;
    factors.push(MatchFactor {
        label: "Commodity".to_string(),
        matched: commodity_match,
        detail: Some(if commodity_match {
            format!("{} matches {}", demand.commodity, listing.commodity)
        } else {
            format!("Requested {}, available {}", demand.commodity, listing.commodity)
        }),
    });
    if !commodity_match {
        return None; // hard filter
    }
    score += 40;

    let quantity_ok = listing.quantity >= demand.quantity;
    factors.push(MatchFactor {
        label: "Quantity available".to_string(),
        matched: quantity_ok,
        detail: Some(if quantity_ok {
            format!("{} {} available, {} {} requested", listing.quantity, listing.unit, demand.quantity, demand.unit)
        } else {
            format!("Only {} {} available", listing.quantity, listing.unit)
        }),
    });
    if quantity_ok {
        score += 20;
    }

    let grade_ok = listing.quality_grade == demand.quality_grade || listing.quality_grade == "A";
    factors.push(MatchFactor {
        label: "Quality grade".to_string(),
        matched: grade_ok,
        detail: Some(if grade_ok {
            format!("Grade {} meets Grade {} requirement", listing.quality_grade, demand.quality_grade)
        } else {
            "Grade mismatch".to_string()
        }),
    });
    if grade_ok {
        score += 15;
    }

    let date_ok = listing.availability_date <= demand.required_by_date;
    factors.push(MatchFactor {
        label: "Availability date".to_string(),
        matched: date_ok,
        detail: Some(if date_ok {
            format!(
                "Available {}, required by {}",
                listing.availability_date.format("%Y-%m-%d"),
                demand.required_by_date.format("%Y-%m-%d")
            )
        } else {
            "Not available until after required date".to_string()
        }),
    });
    if date_ok {
        score += 15;
    }

    let origin = listing.location.split(',').next_back().unwrap_or("").trim().to_lowercase();
    let dest = demand.destination_location.split(',').next_back().unwrap_or("").trim().to_lowercase();
    let location_ok = origin == dest || origin.contains("nigeria") || dest.contains("nigeria");
    factors.push(MatchFactor {
        label: "Destination serviceable".to_string(),
        matched: location_ok,
        detail: Some(if location_ok {
            format!("{} → {}", listing.location, demand.destination_location)
        } else {
            "Locations may be incompatible".to_string()
        }),
    });
    if location_ok {
        score += 10;
    }

    Some((score, factors))
}
