/**
 * Shared "an image-model download is in flight" flag.
 *
 * The Models tab (ai/+page.svelte) and the Image playground
 * (ImagegenPlayground.svelte) each start downloads against the same
 * portbay-imagegen sidecar with their own local guards — concurrent
 * downloads to it are undefined behavior. This single flag gates both
 * surfaces so whichever starts first wins.
 */
export const imagegenDownload = $state({ active: false });
