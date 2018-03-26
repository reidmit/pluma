export default function el(tag, props) {
  props = props || {};
  const key = props.key == null ? null : props.key;
  delete props.key;

  const childrenCount = arguments.length - 2;
  const children = Array(childrenCount);
  let descendantCount = childrenCount;
  for (let i = 0; i < childrenCount; i++) {
    const child = arguments[i + 2];
    children[i] = child;
    descendantCount += child.$d || 0;
  }

  props.children = children;
  return { $t: tag, $p: props, $d: descendantCount, $k: key };
}
