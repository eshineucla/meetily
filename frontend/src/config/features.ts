// Classified build (涉密版, M3-02): set NEXT_PUBLIC_BUILD_VARIANT=classified when building the
// frontend, together with the Rust build's --no-default-features. Cloud summary providers are
// hidden, and the backend refuses them and any model server outside the internal network.
export const CLASSIFIED_BUILD = process.env.NEXT_PUBLIC_BUILD_VARIANT === 'classified';
export const CLOUD_PROVIDERS_ENABLED = !CLASSIFIED_BUILD;

// Stage 1: analytics and auto-update are switched off. The code stays in place
// and is removed from the classified build in stage 3 (M3-02).
export const ANALYTICS_ENABLED = false;
export const UPDATES_ENABLED = false;

// Upstream beta features (audio import and retranscription) are switched off.
export const BETA_FEATURES_ENABLED = false;
