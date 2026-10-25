#!/usr/bin/env python3
"""Owned focused controls for bounded evidence repair; no model calls."""
import copy
import hashlib
import json
from pathlib import Path
import runpy
import tempfile
import unittest

ROOT=Path(__file__).resolve().parent.parent
MODULE=runpy.run_path(str(ROOT/'skills/codeclew/scripts/repair_evidence.py'))
Blocked=MODULE['Blocked']

class RepairTest(unittest.TestCase):
    def setUp(self):
        self.base={'schema':'codeclew-documentation-proposal/1.0','operations':[{
            'entrypoint':'e1','summary':{'text':'Owned fixture','evidence':['d1']},
            'steps':[{'kind':'alt','meaning':{'text':'guard','evidence':['guard']},
                      'children':[{'kind':'return','meaning':{'text':'return null','evidence':['d1']}}],
                      'otherwise':[{'kind':'return','meaning':{'text':'return value','evidence':['d1']}}]}]}],
            'uncertainties':['Unresolved runtime dispatch']}
        self.raw=json.dumps(self.base).encode()
        self.rows=[];targets=[]
        for index,(branch,text) in enumerate([('children','return null'),('otherwise','return value')]):
            record={'kind':'FLOW','normalized':{'kind':'RETURN','text':text,'authority':'SYNTAX','ordinal':index}}
            ref='d'+str(index+2);identifier='owned-flow-'+str(index)
            self.rows.append({'kind':'DEPENDENCY','id':identifier,'reference':ref,'record':record,'sourceReferences':['s1']})
            targets.append({'operation':0,'stepPath':[{'list':'steps','index':0},{'list':branch,'index':0}],
                            'expectedEvidence':['d1'],'expectedText':text,'reference':ref,'recordId':identifier,
                            'recordDigest':MODULE['record_digest'](record),'mapping':'VERBATIM_FLOW_RETURN'})
        self.intent={'schema':'codeclew-evidence-only-repair-intent/1.0','work':'owned-work','snapshot':'sha256:owned/1',
                     'baseDigest':MODULE['digest'](self.raw),'targets':targets}

    def validate(self):
        MODULE['validate_intent'](self.base,self.raw,self.intent)

    def test_exact_two_insertions_preserve_every_other_value_and_original(self):
        before=copy.deepcopy(self.base);self.validate();candidate,bindings=MODULE['materialize'](self.base,self.intent,self.rows)
        for branch in ['children','otherwise']:
            self.assertEqual(len(candidate['operations'][0]['steps'][0][branch][0]['meaning']['evidence']),2)
            candidate['operations'][0]['steps'][0][branch][0]['meaning']['evidence'].pop()
        self.assertEqual(candidate,before);self.assertEqual(self.base,before)
        self.assertEqual([b['authority'] for b in bindings],['SYNTAX','SYNTAX'])

    def test_additional_prose_edit_and_changed_base_are_not_rebased(self):
        self.base['operations'][0]['summary']['text']='Different prose'
        self.raw=json.dumps(self.base).encode()
        with self.assertRaisesRegex(Blocked,'BASE_DIGEST_CHANGED'):self.validate()

    def test_existing_reference_is_not_accepted_as_stale_idempotence(self):
        self.base['operations'][0]['steps'][0]['children'][0]['meaning']['evidence'].append('d2')
        self.raw=json.dumps(self.base).encode();self.intent['baseDigest']=MODULE['digest'](self.raw)
        self.intent['targets'][0]['expectedEvidence']=['d1','d2']
        with self.assertRaisesRegex(Blocked,'REFERENCE_ALREADY_IN_BASE'):self.validate()

    def test_unknown_contract_extra_edit_ambiguous_mapping_and_duplicate_target_fail(self):
        cases=[('schema','unknown'),('approved',True)]
        for field,value in cases:
            intent=copy.deepcopy(self.intent);intent[field]=value
            with self.subTest(field=field),self.assertRaises(Blocked):MODULE['validate_intent'](self.base,self.raw,intent)
        self.intent['targets'][0]['mapping']='INFER_FROM_GENERIC_FLOW_DIAGNOSTIC'
        with self.assertRaisesRegex(Blocked,'NEEDS_CONTEXT_SEMANTIC_MAPPING'):self.validate()
        self.setUp();self.intent['targets'].append(copy.deepcopy(self.intent['targets'][0]))
        with self.assertRaisesRegex(Blocked,'DUPLICATE_TARGET'):self.validate()

    def test_wrong_reference_record_identity_or_authority_promotion_rejected(self):
        for field,value in [('id','foreign'),('kind','SOURCE')]:
            rows=copy.deepcopy(self.rows);rows[0][field]=value
            with self.subTest(field=field),self.assertRaisesRegex(Blocked,'EVIDENCE_REFERENCE_IDENTITY_CHANGED'):
                MODULE['materialize'](self.base,self.intent,rows)
        rows=copy.deepcopy(self.rows);rows[0]['record']['normalized']['authority']='COMPILER'
        with self.assertRaisesRegex(Blocked,'EVIDENCE_RECORD_CHANGED'):MODULE['materialize'](self.base,self.intent,rows)
        rows=copy.deepcopy(self.rows);rows[0]['record']['normalized']['authority']='DECLARED'
        self.intent['targets'][0]['recordDigest']=MODULE['record_digest'](rows[0]['record'])
        with self.assertRaisesRegex(Blocked,'UNSUPPORTED_FLOW_AUTHORITY'):MODULE['materialize'](self.base,self.intent,rows)

    def test_contradictory_or_paraphrased_claim_requires_author_context(self):
        self.base['operations'][0]['steps'][0]['children'][0]['meaning']['text']='return a successful city'
        self.intent['targets'][0]['expectedText']='return a successful city'
        self.raw=json.dumps(self.base).encode();self.intent['baseDigest']=MODULE['digest'](self.raw);self.validate()
        with self.assertRaisesRegex(Blocked,'NEEDS_CONTEXT_NONVERBATIM_MEANING'):
            MODULE['materialize'](self.base,self.intent,self.rows)

    def test_paths_cannot_change_text_guards_or_unknown_fields(self):
        for path in [[{'list':'summary','index':0}],[{'list':'steps','index':True}],[]]:
            intent=copy.deepcopy(self.intent);intent['targets'][0]['stepPath']=path
            with self.subTest(path=path),self.assertRaises(Blocked):MODULE['validate_intent'](self.base,self.raw,intent)
        self.intent['targets'][0]['extraEdit']={'summary':'changed'}
        with self.assertRaisesRegex(Blocked,'INVALID_TARGET_FIELDS'):self.validate()

    def test_duplicate_keys_nonfinite_and_malformed_json_rejected(self):
        for raw in ['{"x":1,"x":2}','{"x":NaN}','{bad']:
            with self.subTest(raw=raw),self.assertRaises(ValueError):MODULE['load'](raw)

    def test_existing_output_is_never_overwritten_or_treated_as_approval(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)/'existing';output.mkdir();(output/'result.json').write_text('keep exact bytes')
            with self.assertRaises(FileExistsError):
                MODULE['repair'](Path('/missing'),Path('/missing'),Path('/missing'),Path('/missing'),output)
            self.assertEqual((output/'result.json').read_text(),'keep exact bytes')

    def test_input_paths_resolve_from_one_assigned_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)/"assigned";root.mkdir();(root/"run03").mkdir()
            expected=root/"run03/proposal.json"
            self.assertEqual(MODULE['anchored'](root,Path('run03/proposal.json')),expected.resolve())
            with self.assertRaisesRegex(Blocked,'INPUT_PATH_OUTSIDE_ASSIGNED_ROOT'):
                MODULE['anchored'](root,Path('../other/proposal.json'))

    def test_non_object_steps_save_blocked_result_and_originals_before_native_reads(self):
        for node in (None, [], ['unexpected'], 'return', 0, True):
            with self.subTest(node=node), tempfile.TemporaryDirectory() as directory:
                root=Path(directory);base=copy.deepcopy(self.base);intent=copy.deepcopy(self.intent)
                base['operations'][0]['steps'][0]['children'][0]=node
                raw=json.dumps(base).encode();intent['baseDigest']=MODULE['digest'](raw)
                intent_raw=json.dumps(intent).encode()
                (root/'base.json').write_bytes(raw);(root/'intent.json').write_bytes(intent_raw)
                output=root/'result'
                result=MODULE['repair'](root/'missing-launcher',root/'docs',root/'base.json',root/'intent.json',output)
                self.assertEqual(result['status'],'BLOCKED')
                self.assertEqual(result['reason'],'TARGET_STEP_NOT_OBJECT')
                self.assertEqual(json.loads((output/'result.json').read_text()),result)
                self.assertEqual((output/'base.original.json').read_bytes(),raw)
                self.assertEqual((output/'intent.original.json').read_bytes(),intent_raw)
                self.assertFalse((output/'evidence-collection').exists())
                self.assertFalse((output/'candidate.json').exists())

if __name__=='__main__':unittest.main()
