<script lang="ts">
  import logoUrl from "$lib/assets/portbay-logo.png";

  interface Props {
    size?: number;
    class?: string;
  }
  let { size = 36, class: cls = "" }: Props = $props();
</script>

<img
  src={logoUrl}
  alt="PortBay"
  width={size}
  height={size}
  class={cls}
  aria-hidden="true"
  style="width:{size}px;height:{size}px;object-fit:contain;"
/>
