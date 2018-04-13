class LinkerError extends Error {
  constructor(message, fileName) {
    super('\n\n' + message);
    this.name = `Linker error${fileName ? " in '" + fileName + "'" : ''}`;
    this.stack = null;
  }
}

export default LinkerError;
