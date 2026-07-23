---
name: Kinetic Studio
colors:
  surface: '#131313'
  surface-dim: '#131313'
  surface-bright: '#393939'
  surface-container-lowest: '#0e0e0e'
  surface-container-low: '#1b1b1c'
  surface-container: '#202020'
  surface-container-high: '#2a2a2a'
  surface-container-highest: '#353535'
  on-surface: '#e5e2e1'
  on-surface-variant: '#dfc0af'
  inverse-surface: '#e5e2e1'
  inverse-on-surface: '#303030'
  outline: '#a68b7b'
  outline-variant: '#584235'
  surface-tint: '#ffb688'
  primary: '#ffb688'
  on-primary: '#512400'
  primary-container: '#ff7f00'
  on-primary-container: '#5e2b00'
  inverse-primary: '#974900'
  secondary: '#c8c6c6'
  on-secondary: '#303030'
  secondary-container: '#474747'
  on-secondary-container: '#b6b5b4'
  tertiary: '#98cbff'
  on-tertiary: '#003354'
  tertiary-container: '#35a9ff'
  on-tertiary-container: '#003c62'
  error: '#ffb4ab'
  on-error: '#690005'
  error-container: '#93000a'
  on-error-container: '#ffdad6'
  primary-fixed: '#ffdbc7'
  primary-fixed-dim: '#ffb688'
  on-primary-fixed: '#311300'
  on-primary-fixed-variant: '#733600'
  secondary-fixed: '#e4e2e1'
  secondary-fixed-dim: '#c8c6c6'
  on-secondary-fixed: '#1b1c1c'
  on-secondary-fixed-variant: '#474747'
  tertiary-fixed: '#cfe5ff'
  tertiary-fixed-dim: '#98cbff'
  on-tertiary-fixed: '#001d33'
  on-tertiary-fixed-variant: '#004a77'
  background: '#131313'
  on-background: '#e5e2e1'
  surface-variant: '#353535'
typography:
  display-lg:
    fontFamily: Inter
    fontSize: 48px
    fontWeight: '700'
    lineHeight: 56px
    letterSpacing: -0.02em
  headline-lg:
    fontFamily: Inter
    fontSize: 32px
    fontWeight: '600'
    lineHeight: 40px
    letterSpacing: -0.01em
  headline-lg-mobile:
    fontFamily: Inter
    fontSize: 24px
    fontWeight: '600'
    lineHeight: 32px
  title-md:
    fontFamily: Inter
    fontSize: 18px
    fontWeight: '600'
    lineHeight: 24px
  body-md:
    fontFamily: Inter
    fontSize: 14px
    fontWeight: '400'
    lineHeight: 20px
  label-sm:
    fontFamily: Geist
    fontSize: 11px
    fontWeight: '500'
    lineHeight: 16px
    letterSpacing: 0.05em
rounded:
  sm: 0.125rem
  DEFAULT: 0.25rem
  md: 0.375rem
  lg: 0.5rem
  xl: 0.75rem
  full: 9999px
spacing:
  unit: 4px
  xs: 4px
  sm: 8px
  md: 16px
  lg: 24px
  xl: 48px
  gutter: 12px
  margin-mobile: 16px
  margin-desktop: 32px
---

## Brand & Style

This design system draws inspiration from high-performance digital audio workstations, specifically the technical and energetic aesthetic of Bitwig Studio. The personality is **precision-engineered, creative, and immersive**. It is designed for power users who require a high-density information environment that remains legible during long sessions of deep focus.

The design style is a blend of **Modern-Technical and Tonal Minimalism**. It avoids decorative flourishes in favor of functional clarity. The UI should feel like a piece of high-end hardware: tactile, responsive, and indestructible. We utilize a "dark-first" philosophy to reduce eye strain and allow the vibrant accent colors to guide the user's intent.

**Target Audience:**
- Developers and Technical Creatives.
- Power users of complex SaaS platforms.
- Professionals requiring high-density data visualization.

**Emotional Response:**
- Focus, control, and creative momentum.

## Colors

The palette is anchored by a deep, neutral graphite foundation to provide maximum contrast for the signature **Bitwig Orange**.

- **Primary (#FF7F00):** Used exclusively for active states, primary actions, and critical status indicators. It represents energy and interaction.
- **Surface Tones:** A tiered system of grey (#121212 to #323232) creates depth without relying on shadows. Higher elevations are represented by lighter grey values.
- **Secondary/Tertiary:** Subdued blues or teals can be used for secondary data streams (like automation lines or secondary metrics) to distinguish them from the primary orange flow.
- **High Contrast:** Text and icons should utilize pure white (#FFFFFF) for primary content and a mid-grey (#A0A0A0) for secondary metadata.

## Typography

This design system utilizes **Inter** for its exceptional legibility in dense interfaces and **Geist** for technical labels and monospaced data points.

- **Headlines:** Use tight letter spacing and bold weights to feel impactful and "locked-in."
- **Body:** Set at 14px for standard desktop density. The line height is kept relatively tight (1.4x) to maintain the technical, high-density feel.
- **Labels:** Technical labels (knob values, small parameters) should use Geist. Use uppercase sparingly for section headers to evoke a hardware-chassis vibe.
- **Mobile scaling:** Headlines scale down aggressively to ensure they don't break the tight layout constraints on smaller screens.

## Layout & Spacing

The layout philosophy follows a **strict 4px grid** to maintain technical alignment. 

- **Density:** The design favors high-density layouts. Vertical spacing is often tighter than horizontal spacing to allow for "stacking" of controls.
- **Fluid Grid:** Content should be organized in a 12-column fluid grid. Gutters are kept narrow (12px) to maximize the "instrument" feel of the UI.
- **Modular Panels:** The layout should be thought of as a series of collapsible or resizable panels rather than a single flowing page.
- **Responsive Behavior:** On mobile, panels stack vertically. On desktop, they are positioned side-by-side to mimic a studio monitor setup.

## Elevation & Depth

In this design system, depth is achieved through **Tonal Layering** and **Stroke Definition** rather than soft shadows.

- **Stacking:** The background is the darkest layer (#121212). Each interactive container or panel is one shade lighter (e.g., #1E1E1E).
- **Hard Edges:** Use 1px solid borders (#353535) to define boundaries between panels.
- **Active State Elevation:** Elements do not "lift" with shadows when hovered; instead, they brighten in tone or gain a high-contrast border in the primary orange color.
- **Inset Effects:** Form inputs and "wells" use a subtle inset border or a slightly darker background than their parent container to appear recessed into the "hardware."

## Shapes

To maintain the hardware-inspired, professional look, shapes are kept **sharp or subtly softened**.

- **Standard Radius:** 4px (Soft) for buttons, inputs, and cards. This provides a modern touch without appearing "bubbly" or consumer-grade.
- **Large Components:** For main application panels, a 0px radius is preferred to emphasize the structural, edge-to-edge nature of the interface.
- **Functional Icons:** Icons should be geometric and maintain a consistent 2px stroke weight to match the technical typography.

## Components

- **Buttons:** 
  - *Primary:* Solid Orange background with Black text. No gradient.
  - *Secondary:* Dark grey background with white text and a 1px border.
  - *Active/Toggle:* When a button is "on," it glows with the primary orange or a high-contrast indicator light.
- **Inputs:** Darker than the surrounding surface. Text is white. Focus state is a 1px primary orange border.
- **Chips/Badges:** Small, rectangular, with the Label-SM typography. Used for tags or status indicators.
- **Lists:** High-density, 32px or 40px row heights. Hover states use a subtle lightening of the background.
- **Cards:** No shadows. Defined by a 1px border (#353535) and a slightly elevated surface color (#252525).
- **Specialized UI:** 
  - *Knobs/Sliders:* Vertical sliders (faders) are preferred. Use the primary orange for the "filled" portion of the track.
  - *Level Meters:* Use segmented blocks rather than smooth gradients for a more digital, precise appearance.

