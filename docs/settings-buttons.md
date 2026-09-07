# Settings buttons

Settings actions use the default button surface so their hit areas are visible.
This includes Revoke, Test, Remove, Cancel and provider rename. Primary actions
and existing danger actions retain their respective variants.

Ghost buttons are limited to navigation or actions embedded in an existing
control surface: the narrow split-view Back button and the copy buttons inside
sign-in code and command boxes. Routine settings actions should not use ghost
merely because they are secondary.

These styles live in `web-ui/settings`, shared by the product and gallery.
