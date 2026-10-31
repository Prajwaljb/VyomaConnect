import json, sys, os
def inline(xs, refs):
    out=[]
    for x in xs or []:
        t=x.get('type')
        if t=='text': out.append(x['text'])
        elif t in('emphasis','strong'): out.append(inline(x.get('inlineContent'),refs))
        elif t=='codeVoice': out.append('`'+x.get('code','')+'`')
        elif t=='reference':
            r=refs.get(x.get('identifier'),{}); out.append(r.get('title',''))
        elif t=='link': out.append(x.get('title',''))
        elif 'inlineContent' in x: out.append(inline(x['inlineContent'],refs))
    return ''.join(out)
def block(b, refs, out):
    t=b.get('type')
    if t=='heading': out.append('\n'+'#'*min(b.get('level',2)+1,5)+' '+b.get('text',''))
    elif t=='paragraph': out.append(inline(b.get('inlineContent'),refs))
    elif t in('unorderedList','orderedList'):
        for it in b.get('items',[]):
            sub=[]
            for c in it.get('content',[]): block(c,refs,sub)
            out.append('- '+' '.join(s.strip() for s in sub))
    elif t=='aside':
        sub=[]
        for c in b.get('content',[]): block(c,refs,sub)
        out.append('> '+' '.join(sub))
    elif t=='table':
        for row in b.get('rows',[]):
            cells=[]
            for cell in row:
                sub=[]
                for c in cell: block(c,refs,sub)
                cells.append(' '.join(sub))
            out.append('| '+' | '.join(cells)+' |')
    elif 'content' in b:
        for c in b['content']: block(c,refs,out)
for f in sys.argv[1:-1]:
    try: d=json.load(open(f))
    except Exception: continue
    refs=d.get('references',{})
    out=['\n## '+d.get('metadata',{}).get('title',os.path.basename(f))]
    for sec in d.get('primaryContentSections',[]):
        for c in sec.get('content',[]): block(c,refs,out)
    open(os.path.join(sys.argv[-1],os.path.basename(f)[:-5]+'.md'),'w').write('\n'.join(out))
