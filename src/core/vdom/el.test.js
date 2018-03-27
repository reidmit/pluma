import el from './el';

xdescribe('el', () => {
  it('returns an object', () => {
    expect(el('h1', null, 'hello, world!')).toEqual({
      $t: 'h1',
      $p: { children: ['hello, world!'] },
      $d: 1,
      $k: null
    });
  });

  it('can be given a key', () => {
    expect(el('h1', { key: 'nice' }, 'hello, world!')).toEqual({
      $t: 'h1',
      $p: { children: ['hello, world!'] },
      $d: 1,
      $k: 'nice'
    });
  });

  it('can take props', () => {
    expect(el('h1', { id: 'some-id' }, 'hello, world!')).toEqual({
      $t: 'h1',
      $p: { id: 'some-id', children: ['hello, world!'] },
      $d: 1,
      $k: null
    });
  });

  it('can take no children', () => {
    expect(el('h1', { id: 'some-id' })).toEqual({
      $t: 'h1',
      $p: { id: 'some-id', children: [] },
      $d: 0,
      $k: null
    });
  });

  it('can take multiple children', () => {
    expect(
      el(
        'h1',
        { id: 'some-id' },
        'hello',
        el('h2', { id: 'some-other-id', key: 'subchild' }),
        'world',
        el('h3', { id: 'something-else' })
      )
    ).toEqual({
      $t: 'h1',
      $p: {
        id: 'some-id',
        children: [
          'hello',
          {
            $t: 'h2',
            $p: { id: 'some-other-id', children: [] },
            $d: 0,
            $k: 'subchild'
          },
          'world',
          {
            $t: 'h3',
            $p: { id: 'something-else', children: [] },
            $d: 0,
            $k: null
          }
        ]
      },
      $d: 4,
      $k: null
    });
  });
});
