// A stylesheet imported as its text (`with { loader: 'text' }`), as the contrast test reads
// tokens.css.
declare module '*.css' {
  const text: string;
  export default text;
}
