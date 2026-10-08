import logo from '../../assets/logo.png';

/**
 * The SDC mark (0.18): the logo's hexagon with its background removed, in one place so the topbar,
 * the empty state, the answer header and About all draw the same picture. `glow` adds the soft halo of
 * its own colours, for the places where the mark stands alone.
 */
export function BrandLogo({ size = 24, glow = false, className = '' }: { size?: number; glow?: boolean; className?: string }) {
  return (
    <img
      src={logo}
      alt=""
      aria-hidden="true"
      draggable={false}
      width={size}
      height={size}
      className={'brand-logo shrink-0 select-none object-contain ' + (glow ? 'logo-glow ' : '') + className}
      style={{ width: size, height: size }}
    />
  );
}
