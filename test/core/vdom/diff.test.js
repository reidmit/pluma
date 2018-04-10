import diff from '../../../src/core/vdom/diff';
import el from '../../../src/core/vdom/el';
import {
  REDRAW_PATCH,
  PROP_CHANGE_PATCH,
  CHILDREN_PATCH
} from '../../../src/core/vdom/constants';

xdescribe('diff', () => {
  test('reference-equal elements', () => {
    const prev = el('h1', null);
    const curr = prev;
    expect(diff(prev, curr)).toEqual([]);
  });

  test('tag changes', () => {
    const prev = el('h1', null);
    const curr = el('div', null);
    expect(diff(prev, curr)).toEqual([
      {
        type: REDRAW_PATCH,
        data: null,
        index: 0
      }
    ]);
  });

  test('no changes', () => {
    const prev = el('div', { id: 'hello' }, el('h1', {}, 'world'));
    const curr = el('div', { id: 'hello' }, el('h1', {}, 'world'));
    expect(diff(prev, curr)).toEqual([]);
  });

  test('prop added', () => {
    const prev = el('h1', null);
    const curr = el('h1', { id: 'lol' });
    expect(diff(prev, curr)).toEqual([
      {
        type: PROP_CHANGE_PATCH,
        data: { id: 'lol' },
        index: 0
      }
    ]);
  });

  test('prop changed', () => {
    const prev = el('h1', { id: 'lol' });
    const curr = el('h1', { id: 'lol2' });
    expect(diff(prev, curr)).toEqual([
      {
        type: PROP_CHANGE_PATCH,
        data: { id: 'lol2' },
        index: 0
      }
    ]);
  });

  test('prop removed', () => {
    const prev = el('h1', { id: 'lol' });
    const curr = el('h1', null, []);
    expect(diff(prev, curr)).toEqual([
      {
        type: PROP_CHANGE_PATCH,
        data: { id: null },
        index: 0
      }
    ]);
  });

  test('child added', () => {
    const prev = el('div', { id: 'lol' });
    const curr = el('div', { id: 'lol' }, el('span', { id: 'child-two' }));
    expect(diff(prev, curr)).toEqual([
      {
        type: CHILDREN_PATCH,
        data: { id: null },
        index: 0
      }
    ]);
  });

  test('child changed', () => {
    const prev = el('div', { id: 'lol' }, el('span', { id: 'child-one' }));
    const curr = el('div', { id: 'lol' }, el('span', { id: 'child-two' }));
    expect(diff(prev, curr)).toEqual([
      {
        type: CHILDREN_PATCH,
        index: 0,
        data: { id: null }
      }
    ]);
  });

  test('child removed', () => {
    const prev = el('div', { id: 'lol' }, el('span', { id: 'child-one' }));
    const curr = el('div', { id: 'lol' });
    expect(diff(prev, curr)).toEqual([
      {
        type: CHILDREN_PATCH,
        data: { id: null },
        index: 0
      }
    ]);
  });

  test('temp', () => {
    let prev, curr, patches;

    prev = el(
      'h1',
      { id: 'lol' },
      el('child', { key: 'a', name: 'reid' }),
      el('child2', { key: 'b', name: 'reid' })
    );
    curr = el(
      'h1',
      { id: 'lol' },
      el('child2', { key: 'b', name: 'reid' }),
      el('child', { key: 'a', name: 'reid' })
    );

    patches = diff(prev, curr);
    console.log(JSON.stringify(patches, null, 2));
  });
});
