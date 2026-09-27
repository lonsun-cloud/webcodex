const markUrl = new URL("./brand-mark.svg", import.meta.url).href;

export function BrandMark() {
  return <img className="brand-mark" src={markUrl} alt="" aria-hidden="true" />;
}
