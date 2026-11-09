import re,pathlib
root=pathlib.Path('app'); lib=(root/'src/library.luau').read_text(); names=set(re.findall(r'function M[.:](\w+)|M\.(\w+)\s*=',lib)); names={a or b for a,b in names}; pairs=set()
for f in (root/'src').rglob('*.rs'):
 for n,c in re.findall(r'\("(\w+)",\s*"(\w+)"\)',f.read_text()):
  if n in names:pairs.add((n,c))
s="\n-- Every discovered library method registration: metadata and invalid self only.\n"
for n,c in sorted(pairs):
 s+=f'''test('library.{c}.{n}',function() local ok,o=pcall(Instance.new,'{c}'); if not ok then o=game:GetService('{c}') end; local f=o['{n}']; info('library.{c}.{n}.info',f); test('library.{c}.{n}.badself',function() return f({{}}) end); return true end)\n'''
s+="test('enum.eq',function() return Enum.Material==Enum.Material,Enum.Material.Plastic==Enum.Material.Plastic,Enum.Material.Plastic==Enum.Material.Wood end)\n"
s+="test('enum.items',function() local t=Enum.Material:GetEnumItems(); return #t,t[1],table.find(t,Enum.Material.Plastic)~=nil end)\n"
s+="test('enums.get',function() return #Enum:GetEnums() end)\n"
s+="test('enum.fromname.absent',function() return Enum.Material:FromName('____________') end)\n"
s+="test('enum.fromvalue.absent',function() return Enum.Material:FromValue(-123) end)\n"
s+="out('EXTENDED_DONE')\n"
p=root/'tools/parity_probe/bridge/probe.luau';p.write_text(p.read_text()+s)
print(len(pairs))
