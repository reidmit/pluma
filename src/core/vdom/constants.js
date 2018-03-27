const hasSymbol = typeof Symbol === 'function' && Symbol['for'];

// export const REDRAW_PATCH = hasSymbol ? Symbol['for']('REDRAW') : 0x471;
export const REDRAW_PATCH = 'redraw';

export const PROP_CHANGE_PATCH = 'prop-change';
// export const PROP_CHANGE_PATCH = hasSymbol
//   ? Symbol['for']('PROP_CHANGE')
//   : 0x472;

export const CHILDREN_PATCH = 'children-change'; //hasSymbol ? Symbol['for']('CHILDREN') : 0x473;
// export const CHILDREN_PATCH = hasSymbol ? Symbol['for']('CHILDREN') : 0x473;

export const INSERT_PATCH = 'insertion';
// export const INSERT_PATCH = hasSymbol ? Symbol['for']('INSERT') : 0x474;

export const REMOVE_PATCH = 'removal';
// export const REMOVE_PATCH = hasSymbol ? Symbol['for']('REMOVE') : 0x475;

export const MOVE_PATCH = 'move';
// export const MOVE_PATCH = hasSymbol ? Symbol['for']('MOVE') : 0x476;
