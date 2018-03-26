import {
  REDRAW_PATCH,
  PROP_CHANGE_PATCH,
  CHILDREN_PATCH,
  INSERT_PATCH,
  REMOVE_PATCH,
  MOVE_PATCH
} from './constants';

function insertNode(changes, subpatches, key, node, newIndex, inserts) {
  let entry = changes[key];

  if (key == null || !entry) {
    // Never seen this key before
    entry = {
      type: INSERT_PATCH,
      node,
      index: newIndex,
      data: null
    };

    inserts.push({ index: newIndex, entry });
    if (key != null) changes[key] = entry;

    return;
  }

  if (entry && entry.type === REMOVE_PATCH) {
    inserts.push({ index: newIndex, entry });

    entry.type = MOVE_PATCH;
    entry.index = newIndex;
    const innerPatches = [];
    diffNodes(entry.node, node, innerPatches, entry.index);
    entry.data.data = {
      patches: innerPatches,
      entry
    };

    return;
  }

  insertNode(
    changes,
    subpatches,
    (key || '') + '_suffix',
    node,
    newIndex,
    inserts
  );
}

function removeNode(changes, subpatches, key, node, index) {
  let entry = changes[key];

  if (key == null || !entry) {
    const patch = {
      type: REMOVE_PATCH,
      index
    };

    subpatches.push(patch);

    if (key != null) {
      changes[key] = {
        type: REMOVE_PATCH,
        node,
        index,
        data: patch
      };
    }

    return;
  }

  if (entry && entry.type === INSERT_PATCH) {
    entry.type = MOVE_PATCH;

    const innerPatches = [];
    diffNodes(node, entry.node, innerPatches, index);
    const patch = {
      type: REMOVE_PATCH,
      index,
      data: {
        patches: innerPatches,
        entry
      }
    };

    subpatches.push(patch);

    return;
  }

  removeNode(changes, subpatches, (key || '') + '_suffix', node, index);
}

function diffChildren(oldChildren, newChildren, parentIndex) {
  console.log('diffChildren');
  console.log({ oldChildren, newChildren, parentIndex });

  const localPatches = [];

  const changes = {};
  const inserts = {};

  const oldLength = oldChildren.length;
  const newLength = newChildren.length;

  let oldIndex = 0;
  let newIndex = 0;
  let index = parentIndex;

  while (oldIndex < oldLength && newIndex < newLength) {
    const oldChild = oldChildren[oldIndex];
    const newChild = newChildren[newIndex];

    const oldKey = oldChild.$k;
    const newKey = newChild.$k;

    if (oldKey != null && oldKey === newKey) {
      index++;
      diffNodes(oldChild, newChild, localPatches, index);
      index += oldChild.$d;
      oldIndex++;
      newIndex++;
      continue;
    }

    const oldNext = oldChildren[oldIndex + 1];
    const newNext = newChildren[newIndex + 1];

    let oldNextKey;
    let oldMatch;
    let newNextKey;
    let newMatch;

    if (oldNext) {
      oldNextKey = oldNext.key;
      oldMatch = newKey === oldNextKey;
    }

    if (newNext) {
      newNextKey = newNext.key;
      newMatch = oldKey === newNextKey;
    }

    console.log({ oldMatch, newMatch });

    if (newMatch && oldMatch) {
      // old and new children have swapped places

      index++;
      diffNodes(oldChild, newNext, localPatches, index);
      insertNode(changes, localPatches, oldKey, newChild, newIndex, inserts);
      index += oldChild.$d;

      index++;
      removeNode(changes, localPatches, oldKey, oldNext, index);
      index += oldNext.$d;

      oldIndex += 2;
      newIndex += 2;
      continue;
    }

    if (newMatch) {
      // insert new

      index++;
      insertNode(changes, localPatches, newKey, newChild, newIndex, inserts);
      diffNodes(oldChild, newNext, localPatches, index);
      index += oldChild.$d;

      oldIndex += 1;
      newIndex += 2;
      continue;
    }

    if (oldMatch) {
      // remove old

      index++;
      removeNode(changes, localPatches, oldKey, oldChild, index);
      index += oldChild.$d;

      index++;
      diffNodes(oldNext, newChild, localPatches, index);
      index += oldNext.$d;

      oldIndex += 2;
      newIndex += 1;
      continue;
    }

    if (oldNext && oldNextKey != null && oldNextKey === newNextKey) {
      // remove old, insert new

      index++;
      removeNode(changes, localPatches, oldKey, oldChild, index);
      insertNode(changes, localPatches, newKey, newChild, newIndex, inserts);
      index += oldChild.$d;

      index++;
      diffNodes(oldNext, newNext, localPatches, index);
      index += oldNext.$d;

      oldIndex += 2;
      newIndex += 2;
      continue;
    }

    break;
  }

  while (oldIndex < oldLength) {
    index++;
    const oldCurr = oldChildren[oldIndex];
    removeNode(changes, localPatches, oldCurr.$k, oldCurr, index);
    index += oldCurr.$d;
    oldIndex++;
  }

  const endInserts = [];
  while (newIndex < newLength) {
    index++;
    const newCurr = newChildren[newIndex];
    insertNode(
      changes,
      localPatches,
      newCurr.$k,
      newCurr,
      undefined,
      endInserts
    );
    newIndex++;
  }

  if (localPatches.length > 0 || inserts.length > 0 || endInserts.length > 0) {
    return {
      subpatches: localPatches,
      inserts,
      endInserts
    };
  }
}

function diffProps(oldProps, newProps, patches, index) {
  const propDiff = { removed: [], changed: [] };
  let changed = false;
  let removed = false;

  for (let oldProp in oldProps) {
    if (!(oldProp in newProps)) {
      removed = true;
      propDiff.removed.push(oldProp);
    }
  }

  for (let newProp in newProps) {
    if (!(newProp in oldProps)) {
      changed = true;
      propDiff.changed.push([newProp, newProps[newProp]]);
    }

    if (newProp === 'children') {
      const childrenDiff = diffChildren(
        oldProps.children,
        newProps.children,
        index
      );

      if (childrenDiff) {
        patches.push({
          type: CHILDREN_PATCH,
          index,
          data: childrenDiff
        });
      }

      continue;
    }

    if (newProps[newProp] !== oldProps[newProp]) {
      changed = true;
      propDiff.changed.push([newProp, newProps[newProp]]);
    }
  }

  return changed || removed ? propDiff : null;
}

function diffNodes(oldNode, newNode, patches, index) {
  if (oldNode === newNode) return;

  console.log('diffNodes!');
  console.log({ oldNode, newNode });

  if (oldNode.$t !== newNode.$t) {
    // If the tag changed, re-create the whole node
    patches.push({
      type: REDRAW_PATCH,
      index,
      data: null
    });
    return;
  }

  const propDiff = diffProps(oldNode.$p, newNode.$p, patches, index);
  if (propDiff) {
    patches.push({
      type: PROP_CHANGE_PATCH,
      index,
      changes: propDiff
    });
  }
}

export default function diff(oldNode, newNode) {
  const patches = [];
  diffNodes(oldNode, newNode, patches, 0);
  return patches;
}
