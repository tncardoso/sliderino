# Build the full site: the book first, then Hugo.
#
# The order is important. mdbook renders into site/static/docs/, and Hugo
# copies that tree into site/public/docs/ when it builds. If Hugo runs first,
# the docs are missing or old.
site *ARGS:
    mdbook build
    # --cleanDestinationDir because mdbook fingerprints its stylesheets
    # (sliderino-<hash>.css). Without it, each edit leaves the previous hash
    # in site/public/, and the deploy sends it as dead weight.
    cd site && hugo --minify --cleanDestinationDir {{ARGS}}

# The same site, served on this machine.
serve:
    mdbook build
    cd site && hugo server
