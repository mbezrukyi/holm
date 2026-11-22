//! A row of items on one line. The items that do not fit fold into one "more" item.
//!
//! The row draws a lead, the items, the "more" item and a tail, from left to right. The tail
//! sits at the right end, and never over another part. An item that does not fit whole
//! folds, and so do all items after it. The fold follows the width of the row in each frame.

use std::ops::Range;

use gpui_kit::{
    AlignItems, AnyElement, App, AvailableSpace, Bounds, Display, Element, ElementId, FlexDirection, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Overflow, ParentElement as _, Pixels, Style, Styled as _, Window, div,
    point, px, relative, size,
};

/// One part of the row, with its layout in a row with no fold.
struct Part {
    element: AnyElement,
    layout: LayoutId,
}

pub(crate) struct Fold {
    gap: Pixels,
    lead: Option<AnyElement>,
    items: Vec<AnyElement>,
    /// Builds the "more" item for the folded items.
    more: Box<dyn Fn(Range<usize>) -> AnyElement>,
    tail: Option<AnyElement>,
}

impl Fold {
    /// `more` builds the "more" item for the items in its range.
    pub(crate) fn new(
        gap: Pixels,
        items: impl IntoIterator<Item = AnyElement>,
        more: impl Fn(Range<usize>) -> AnyElement + 'static,
    ) -> Self {
        Self {
            gap,
            lead: None,
            items: items.into_iter().collect(),
            more: Box::new(more),
            tail: None,
        }
    }

    /// The item before all others. It never folds.
    pub(crate) fn lead(mut self, lead: Option<impl IntoElement>) -> Self {
        self.lead = lead.map(IntoElement::into_any_element);
        self
    }

    /// The item at the right end. It never folds.
    pub(crate) fn tail(mut self, tail: Option<impl IntoElement>) -> Self {
        self.tail = tail.map(IntoElement::into_any_element);
        self
    }
}

/// The parts in the order of the row.
pub(crate) struct Parts {
    lead: Option<Part>,
    items: Vec<Part>,
    tail: Option<Part>,
}

/// Prepaints `part` with its left edge at `x`.
fn place(part: Part, x: Pixels, window: &mut Window, cx: &mut App) -> AnyElement {
    let mut element = part.element;
    let natural = window.layout_bounds(part.layout).origin.x;
    window.with_element_offset(point(x - natural, px(0.)), |window| {
        element.prepaint(window, cx);
    });
    element
}

impl IntoElement for Fold {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Fold {
    type RequestLayoutState = Parts;
    type PrepaintState = Vec<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Every part keeps its own width, so the row gives the widths before any fold.
        let mut part = |element: AnyElement| {
            let mut element = div().flex_none().child(element).into_any_element();
            let layout = element.request_layout(window, cx);
            Part { element, layout }
        };
        let parts = Parts {
            lead: self.lead.take().map(&mut part),
            items: self.items.drain(..).map(&mut part).collect(),
            tail: self.tail.take().map(&mut part),
        };
        let children: Vec<LayoutId> = parts
            .lead
            .iter()
            .chain(&parts.items)
            .chain(&parts.tail)
            .map(|part| part.layout)
            .collect();
        let mut style = Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            ..Style::default()
        };
        style.gap.width = self.gap.into();
        style.size.width = relative(1.).into();
        // So the parts past the right end do not make the row wider.
        style.overflow.x = Overflow::Hidden;
        (window.request_layout(style, children, cx), parts)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        parts: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let width = |part: &Part, window: &mut Window| window.layout_bounds(part.layout).size.width;
        let lead = parts.lead.as_ref().map(|part| width(part, window));
        let tail = parts.tail.as_ref().map(|part| width(part, window));
        let items: Vec<Pixels> = parts.items.iter().map(|part| width(part, window)).collect();
        let total = items.len();
        let gap = self.gap;

        // Whether the lead, `shown` items, the "more" item and the tail fit, with gaps.
        let fits = |shown: usize, more: Option<Pixels>| {
            let width = lead
                .into_iter()
                .chain(items[..shown].iter().copied())
                .chain(more)
                .chain(tail)
                .fold(-gap, |sum, width| sum + gap + width);
            width <= bounds.size.width
        };
        let mut shown = total;
        let mut more = None;
        if !fits(total, None) {
            let space = size(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
            // With no room even for the first item, the row shows only the "more" item.
            shown = (0..total)
                .rev()
                .find(|&shown| {
                    let mut element = (self.more)(shown..total);
                    let size = element.layout_as_root(space, window, cx);
                    more = Some((element, size));
                    fits(shown, Some(size.width))
                })
                .unwrap_or(0);
        }

        let mut drawn = Vec::new();
        let mut x = bounds.left();
        if let Some(part) = parts.lead.take() {
            drawn.push(place(part, x, window, cx));
            x += lead.unwrap_or_default() + gap;
        }
        for (part, width) in parts.items.drain(..).zip(items).take(shown) {
            drawn.push(place(part, x, window, cx));
            x += width + gap;
        }
        if let Some((mut element, more)) = more {
            let y = bounds.top() + (bounds.size.height - more.height) / 2.;
            element.prepaint_at(point(x, y), window, cx);
            drawn.push(element);
            x += more.width + gap;
        }
        if let Some(part) = parts.tail.take() {
            // In a row too narrow for it, the tail goes past the right end, not over a part.
            let x = x.max(bounds.right() - tail.unwrap_or_default());
            drawn.push(place(part, x, window, cx));
        }
        drawn
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _parts: &mut Self::RequestLayoutState,
        drawn: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        for element in drawn {
            element.paint(window, cx);
        }
    }
}
